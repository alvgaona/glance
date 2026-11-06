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

    /// Each hunk's first line in the new file and what it's in, for the sidebar.
    pub(super) fn outline(&self) -> Vec<(u32, String)> {
        self.hunks.iter().map(|h| (h.new_start, h.context.clone())).collect()
    }

    /// The hunk drawn on `line`, if any.
    pub(super) fn hunk_at(&self, line: usize) -> Option<usize> {
        self.ranges.iter().position(|r| r.contains(&line))
    }
}

static EMPTY: View = View { entry: usize::MAX, lines: Vec::new(), ranges: Vec::new(), hunks: Vec::new() };

/// Lines picked inside one hunk (`v`): indices into its `lines`, `anchor..=cursor`.
/// It belongs to one entry and hunk, and lapses when either changes.
pub(super) struct Pick {
    entry: usize,
    hunk: usize,
    anchor: usize,
    pub(super) cursor: usize,
}

impl Pick {
    /// The picked range, changed lines and the context between them.
    pub(super) fn range(&self) -> std::ops::RangeInclusive<usize> {
        self.anchor.min(self.cursor)..=self.anchor.max(self.cursor)
    }
}

/// Title rows, then the entry's half of the change, hunk by hunk.
fn build(repo: &gix::Repository, entry: usize, e: &Entry) -> View {
    let what = match e.group {
        Group::Conflicts => super::conflict::what(e.conflict),
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
    if e.group == Group::Conflicts {
        view.lines.extend(super::conflict::body(repo, e));
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
    /// Builds the diff of the entry under the cursor, unless it's built already.
    pub(super) fn load_view(&mut self) {
        if self.diff.as_ref().map(|v| v.entry) != Some(self.selected) {
            let view = match self.current() {
                Some(e) => build(&self.repo, self.selected, e),
                None => View { entry: self.selected, lines: Vec::new(), ranges: Vec::new(), hunks: Vec::new() },
            };
            self.diff = Some(view);
        }
    }

    /// The diff of the entry under the cursor ([`App::load_view`] builds it before each
    /// draw), or nothing yet.
    pub(super) fn view(&self) -> &View {
        self.diff.as_ref().filter(|v| v.entry == self.selected).unwrap_or(&EMPTY)
    }

    /// Moves the keys to the diff, on its first hunk.
    pub(super) fn enter_hunks(&mut self) {
        let Some(e) = self.current() else { return };
        let why = match e.group {
            Group::Conflicts => Some("A conflict is fixed in the file: e opens it, then space marks it resolved"),
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
    fn current_hunk(&self) -> Option<(String, Hunk)> {
        let path = self.current()?.path.clone();
        let i = self.hunk;
        let hunk = self.view().hunks.get(i)?.clone();
        Some((path, hunk))
    }

    /// Space in the diff: the hunk into the index, or out of it.
    pub(super) fn toggle_hunk(&mut self) -> Option<Effect> {
        let e = self.current()?;
        if whole_file(e) {
            // The file moves to another group, so the keys go back to the list.
            self.focus = Focus::Files;
            return self.toggle();
        }
        let action = if e.group == Group::Staged { HunkAction::Unstage } else { HunkAction::Stage };
        let (path, hunk) = self.current_hunk()?;
        let msg = if action == HunkAction::Stage { "Staged the hunk" } else { "Unstaged the hunk" };
        Some(Effect::Hunk { path, hunk, action, msg: msg.into() })
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
        let msg = "Discarded the hunk".into();
        self.confirm = Some((confirm, Effect::Hunk { path, hunk, action: HunkAction::Discard, msg }));
    }

    /// The line pick, if it still belongs to the hunk in the diff.
    pub(super) fn pick(&self) -> Option<&Pick> {
        self.pick.as_ref().filter(|p| self.focus == Focus::Hunks && p.entry == self.selected && p.hunk == self.hunk)
    }

    /// The picked lines as view rows (the hunk's `@@` row comes first).
    pub(super) fn picked_rows(&self) -> Option<(usize, std::ops::RangeInclusive<usize>)> {
        let (cursor, range) = self.pick().map(|p| (p.cursor, p.range()))?;
        let i = self.hunk;
        let start = self.view().ranges.get(i)?.start + 1;
        Some((start + cursor, start + range.start()..=start + range.end()))
    }

    /// How many changed lines are picked.
    pub(super) fn picked_changes(&self) -> usize {
        let Some(range) = self.pick().map(Pick::range) else { return 0 };
        let i = self.hunk;
        let Some(h) = self.view().hunks.get(i) else { return 0 };
        range.filter(|&j| h.lines.get(j).is_some_and(|l| l.op != Op::Context)).count()
    }

    /// `v`: starts picking lines on the hunk's first change, or stops.
    pub(super) fn toggle_pick(&mut self) {
        if self.pick().is_some() {
            self.pick = None;
            return;
        }
        let Some(e) = self.current() else { return };
        if whole_file(e) {
            self.toast = Some(Toast::new("This file goes whole: space stages all of it", theme::YELLOW));
            return;
        }
        let Some((_, hunk)) = self.current_hunk() else { return };
        let Some(first) = hunk.lines.iter().position(|l| l.op != Op::Context) else { return };
        self.pick = Some(Pick { entry: self.selected, hunk: self.hunk, anchor: first, cursor: first });
        self.show_pick();
    }

    /// j/k while picking: the cursor to the next changed line that way, alone; with
    /// `extend` (J/K) the pick grows or shrinks from where it started.
    pub(super) fn step_pick(&mut self, down: bool, extend: bool) {
        let Some((_, hunk)) = self.current_hunk() else { return };
        let Some(p) = self.pick.as_mut() else { return };
        let changed = |i: &usize| hunk.lines[*i].op != Op::Context;
        let next =
            if down { (p.cursor + 1..hunk.lines.len()).find(changed) } else { (0..p.cursor).rev().find(changed) };
        p.cursor = next.unwrap_or(p.cursor);
        if !extend {
            p.anchor = p.cursor;
        }
        self.show_pick();
    }

    fn show_pick(&mut self) {
        let height = self.detail_area.height.saturating_sub(2);
        if let Some((row, _)) = self.picked_rows() {
            self.detail_scroll.show(row, row, height);
        }
    }

    /// The current hunk cut down to the picked lines for `action`, and how many changed
    /// lines that is; `None` (with a toast) when nothing changed is picked.
    fn picked_hunk(&mut self, action: HunkAction) -> Option<(String, Hunk, usize)> {
        let range = self.pick()?.range();
        let (path, hunk) = self.current_hunk()?;
        let picked: Vec<usize> = range.filter(|&i| hunk.lines[i].op != Op::Context).collect();
        let part = stage::pick_lines(&hunk, &picked, action);
        if !stage::changes_anything(&part) {
            self.toast = Some(Toast::new("No changed lines picked", theme::YELLOW));
            return None;
        }
        Some((path, part, picked.len()))
    }

    /// Space while picking: the picked lines into the index, or out of it.
    pub(super) fn toggle_lines(&mut self) -> Option<Effect> {
        let staged = self.current().is_some_and(|e| e.group == Group::Staged);
        let action = if staged { HunkAction::Unstage } else { HunkAction::Stage };
        let (path, hunk, n) = self.picked_hunk(action)?;
        let done = if staged { "Unstaged" } else { "Staged" };
        Some(Effect::Hunk { path, hunk, action, msg: format!("{done} {}", lines(n)) })
    }

    /// d while picking: asks before throwing the picked lines away.
    pub(super) fn ask_discard_lines(&mut self) {
        if self.current().is_some_and(|e| e.group == Group::Staged) {
            self.toast = Some(Toast::new("Those are staged: space unstages them first", theme::YELLOW));
            return;
        }
        let Some((path, hunk, n)) = self.picked_hunk(HunkAction::Discard) else { return };
        let confirm = Confirm {
            title: format!("Discard {} of {path}?", lines(n)),
            body: "They go back to what's staged.\nThe rest of the file stays as it is.".into(),
            yes: "Discard".into(),
            no: "Cancel".into(),
            danger: true,
        };
        let msg = format!("Discarded {}", lines(n));
        self.confirm = Some((confirm, Effect::Hunk { path, hunk, action: HunkAction::Discard, msg }));
    }

    /// After a hunk write: report, reload, and stay on the same file's next hunk if it's
    /// still in the same group; otherwise back to the file list where it was.
    pub(super) fn after_hunk(&mut self, ran: Ran, msg: String) -> Result<()> {
        self.toast = Some(Toast::ran(&ran, msg));
        self.pick = None;
        let Some((group, pos)) = self.place() else { return self.reload() };
        let path = self.current().map(|e| e.path.clone()).unwrap_or_default();
        let hunk = self.hunk;
        self.reload()?;
        match self.entries.iter().position(|e| e.group == group && e.path == path) {
            Some(i) => {
                self.selected = i;
                self.load_view();
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

/// "1 line", "3 lines".
fn lines(n: usize) -> String {
    if n == 1 { "1 line".into() } else { format!("{n} lines") }
}
