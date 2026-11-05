//! The diff pane, hunk by hunk: building it, and staging, unstaging or discarding the
//! hunk under the cursor.

use std::ops::Range;

use super::*;
use crate::git::diff::Op;
use crate::git::stage::{Hunk, HunkAction};

/// The diff of one entry, drawn once and kept until the cursor or the files change.
pub(super) struct View {
    /// The entry it was built for.
    entry: usize,
    pub(super) lines: Vec<Line<'static>>,
    /// Where each hunk is in `lines`.
    pub(super) ranges: Vec<Range<usize>>,
    hunks: Vec<Hunk>,
}

impl View {
    pub(super) fn has_hunks(&self) -> bool {
        !self.ranges.is_empty()
    }

    /// The hunk drawn on `line`, if any.
    pub(super) fn hunk_at(&self, line: usize) -> Option<usize> {
        self.ranges.iter().position(|r| r.contains(&line))
    }
}

/// Title rows, then the entry's half of the change, hunk by hunk.
fn build(repo: &gix::Repository, entry: usize, e: &Entry) -> View {
    let what = match e.group {
        Group::Conflicts => "conflicted: the file as it is now",
        Group::Staged => "staged",
        Group::Changes => "not staged",
        Group::Untracked => "new, not tracked yet",
    };
    let mut title =
        vec![Span::styled(e.path.clone(), theme::bold()), Span::styled(format!("   {what}"), theme::faint())];
    let c = changes::counts(e.size);
    if !c.is_empty() {
        title.push(Span::raw("   "));
        title.extend(c);
    }
    let mut lines = vec![Line::from(title)];
    if let Some(from) = &e.from {
        lines.push(Line::styled(format!("renamed from {from}"), Style::new().fg(theme::BLUE)));
    }
    lines.push(Line::raw(""));
    let mut view = View { entry, lines, ranges: Vec::new(), hunks: Vec::new() };

    if e.is_folder() {
        view.lines.push(Line::styled("A new folder: space stages everything in it.", theme::muted()));
        return view;
    }
    match stage::hunks(repo, &e.path, e.from.as_deref(), e.group.side()) {
        Ok(Some(h)) if h.is_empty() => {
            view.lines.push(Line::styled("No line changes (only the file mode, or an empty file).", theme::muted()))
        }
        Ok(Some(hunks)) => {
            for h in &hunks {
                let start = view.lines.len();
                view.lines.extend(h.patch_lines().into_iter().map(diff::line));
                view.ranges.push(start..view.lines.len());
            }
            view.hunks = hunks;
        }
        Ok(None) => view.lines.push(Line::styled("Binary or very large file.", theme::muted())),
        Err(err) => view.lines.push(Line::styled(format!("Couldn't read it: {err}"), Style::new().fg(theme::RED))),
    }
    view
}

/// Files that are staged or thrown away whole: one side is empty, and `git apply`
/// would leave an empty file instead of adding or removing it.
fn whole_file(e: &Entry) -> bool {
    matches!(e.group, Group::Untracked | Group::Conflicts)
        || matches!(e.kind, Some(ChangeKind::Added | ChangeKind::Deleted))
}

impl App {
    /// The diff of the entry under the cursor.
    pub(super) fn view(&mut self) -> &View {
        if self.diff.as_ref().map(|v| v.entry) != Some(self.selected) {
            let view = match self.current() {
                Some(e) => build(&self.repo, self.selected, e),
                None => View { entry: self.selected, lines: Vec::new(), ranges: Vec::new(), hunks: Vec::new() },
            };
            self.diff = Some(view);
        }
        self.diff.as_ref().unwrap()
    }

    /// Moves the keys to the diff, on its first hunk.
    pub(super) fn enter_hunks(&mut self) {
        let Some(e) = self.current() else { return };
        let why = match e.group {
            Group::Conflicts => Some("Fix the conflict in the file, then space marks it resolved"),
            _ if e.is_folder() => Some("A new folder: space stages all of it"),
            _ => None,
        };
        if let Some(why) = why {
            self.toast = Some(Toast::new(why, theme::YELLOW));
            return;
        }
        if !self.view().has_hunks() {
            self.toast = Some(Toast::new("No hunks here: space stages the whole file", theme::YELLOW));
            return;
        }
        self.focus = Focus::Hunks;
        self.hunk = self.hunk.min(self.view().ranges.len() - 1);
        self.show_hunk();
    }

    pub(super) fn step_hunk(&mut self, n: isize) {
        let last = self.view().ranges.len().saturating_sub(1) as isize;
        self.hunk = (self.hunk as isize + n).clamp(0, last) as usize;
        self.show_hunk();
    }

    /// Scrolls the diff so the current hunk is in view.
    fn show_hunk(&mut self) {
        let (height, i) = (self.detail_area.height.saturating_sub(2), self.hunk);
        if let Some(r) = self.view().ranges.get(i).cloned() {
            // One line above it too, when there's room.
            self.detail_scroll.show(r.start.saturating_sub(1), r.end - 1, height);
        }
    }

    /// The current hunk and what space would do with it.
    fn current_hunk(&mut self) -> Option<(String, Hunk)> {
        let path = self.current()?.path.clone();
        let i = self.hunk;
        let hunk = self.view().hunks.get(i)?.clone();
        Some((path, hunk))
    }

    /// Space in the diff: the hunk into the index, or out of it.
    pub(super) fn toggle_hunk(&mut self) -> Result<()> {
        let Some(e) = self.current() else { return Ok(()) };
        if whole_file(e) {
            // The file moves to another group, so the keys go back to the list.
            self.focus = Focus::Files;
            return self.toggle();
        }
        let action = if e.group == Group::Staged { HunkAction::Unstage } else { HunkAction::Stage };
        let Some((path, hunk)) = self.current_hunk() else { return Ok(()) };
        let r = stage::apply_hunk(&self.repo, &path, &hunk, action);
        let done = if action == HunkAction::Stage { "Staged the hunk" } else { "Unstaged the hunk" };
        self.after_hunk(r, done.into())
    }

    /// d in the diff: asks before throwing the hunk away.
    pub(super) fn ask_discard_hunk(&mut self) {
        let Some(e) = self.current() else { return };
        if e.group == Group::Staged {
            self.toast = Some(Toast::new("That's staged: space unstages the hunk first", theme::YELLOW));
            return;
        }
        if whole_file(e) {
            self.focus = Focus::Files;
            return self.ask_discard();
        }
        let Some((path, hunk)) = self.current_hunk() else { return };
        let count = |op| hunk.lines.iter().filter(|l| l.op == op).count();
        let (added, removed) = (count(Op::Added), count(Op::Removed));
        // The first line that changed, as numbered in the file now.
        let at = hunk.lines.iter().find(|l| l.op != Op::Context).map_or(hunk.new_start, |l| match l.op {
            Op::Removed => l.new + 1,
            _ => l.new,
        });
        let confirm = Confirm {
            title: format!("Discard this hunk of {path}?"),
            body: format!(
                "Its +{added} −{removed} at line {at} go back to what's staged.\nThe rest of the file stays as it is."
            ),
            yes: "Discard".into(),
            no: "Cancel".into(),
            danger: true,
        };
        self.confirm = Some((confirm, Ask::DiscardHunk { path, hunk }));
    }

    /// After a hunk write: report, reload, and stay on the same file's next hunk if it's
    /// still in the same group; otherwise back to the file list where it was.
    pub(super) fn after_hunk(&mut self, (ok, out): (bool, String), msg: String) -> Result<()> {
        self.toast = Some(if ok { Toast::new(msg, theme::GREEN) } else { Toast::new(first_lines(&out), theme::RED) });
        let Some((group, pos)) = self.place() else { return self.reload() };
        let path = self.current().map(|e| e.path.clone()).unwrap_or_default();
        let hunk = self.hunk;
        self.reload()?;
        match self.entries.iter().position(|e| e.group == group && e.path == path) {
            Some(i) => {
                self.selected = i;
                let n = self.view().ranges.len();
                if n == 0 {
                    self.focus = Focus::Files;
                } else {
                    self.hunk = hunk.min(n - 1);
                    self.show_hunk();
                }
            }
            None => {
                self.focus = Focus::Files;
                self.put_at(group, pos);
            }
        }
        Ok(())
    }
}
