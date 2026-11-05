//! `gg add`: every uncommitted file, grouped (conflicts, staged, changes, untracked),
//! with the diff of the one under the cursor. Stage, unstage and discard whole files.
//! Writes go through `git::stage`.

mod commit;
mod conflict;
mod draw;
mod hunks;
mod keys;
#[cfg(test)]
mod tests;

use commit::CommitBox;
use draw::draw;
use hunks::{Pick, View};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};

use crate::git::diff::Size;
use std::collections::HashMap;

use crate::git::run::Ran;
use crate::git::stage::{self, Conflict, Side};
use crate::git::status::{ChangeKind, FileChange, Status};
use crate::tui::app::{self, Flow, Screen};
use crate::tui::kit::{self, Command, Confirm, Palette, PaletteEvent, Scroll, Toast, footer, list};
use crate::tui::panels::{changes, diff};
use crate::tui::{first_lines, theme};

#[derive(Clone)]
enum Cmd {
    Toggle,
    Hunks,
    Commit,
    Amend,
    StageAll,
    UnstageAll,
    Discard,
    Edit,
    Copy,
    Reload,
    Quit,
}

/// What the screen does outside its own state; [`Screen::apply`] does it.
enum Effect {
    Reload,
    /// A write to one whole file, then a toast with `msg`.
    File {
        op: FileOp,
        path: String,
        msg: String,
    },
    StageAll,
    UnstageAll,
    /// A hunk (or the picked lines of one) into the index, out of it, or thrown away.
    Hunk {
        path: String,
        hunk: stage::Hunk,
        action: stage::HunkAction,
        msg: String,
    },
    Copy(String),
    /// Opens the commit box with the draft, git's merge message, or HEAD's message.
    OpenCommit {
        amend: bool,
    },
    Commit {
        message: String,
        amend: bool,
        edit: bool,
    },
    /// A commit that runs with the terminal: signing may prompt, or the editor opens.
    CommitOnTerminal {
        message: String,
        amend: bool,
        edit: bool,
    },
    Edit(String),
    EditOnTerminal(String),
}

#[derive(Clone, Copy)]
enum FileOp {
    Add,
    Unstage,
    Discard,
    DeleteUntracked,
    Remove,
}

/// Which pane the keys work on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Files,
    /// The diff, hunk by hunk.
    Hunks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Conflicts,
    Staged,
    Changes,
    Untracked,
}

impl Group {
    fn title(self) -> &'static str {
        match self {
            Group::Conflicts => "conflicts",
            Group::Staged => "staged",
            Group::Changes => "changes",
            Group::Untracked => "untracked",
        }
    }

    /// Which half of the change the diff shows.
    fn side(self) -> Side {
        if self == Group::Staged { Side::Staged } else { Side::Unstaged }
    }
}

/// One row: a file in one group. A file staged in part is in two groups.
struct Entry {
    group: Group,
    path: String,
    from: Option<String>,
    kind: Option<ChangeKind>,
    size: Option<Size>,
    /// For a conflicted file: which sides still have it.
    conflict: Option<Conflict>,
    /// For a conflicted file: whether it still has `<<<<<<<` lines.
    markers: bool,
}

impl Entry {
    fn change(group: Group, c: &FileChange) -> Self {
        Entry {
            group,
            path: c.path.clone(),
            from: c.from.clone(),
            kind: Some(c.kind),
            size: c.size,
            conflict: None,
            markers: false,
        }
    }

    fn bare(group: Group, path: &str) -> Self {
        Entry { group, path: path.to_string(), from: None, kind: None, size: None, conflict: None, markers: false }
    }

    fn item(&self) -> changes::Item<'_> {
        match (self.group, self.kind) {
            (Group::Conflicts, _) => changes::Item::conflict(&self.path),
            (Group::Untracked, _) | (_, None) => changes::Item::untracked(&self.path),
            (_, Some(k)) => changes::Item {
                mark: changes::kind_mark(k),
                path: &self.path,
                from: self.from.as_deref(),
                size: self.size,
            },
        }
    }

    fn is_folder(&self) -> bool {
        self.path.ends_with('/')
    }
}

fn entries(repo: &gix::Repository, st: &Status, conflicts: &HashMap<String, Conflict>) -> Vec<Entry> {
    let conflicted = st.conflicted.iter().map(|p| Entry {
        conflict: conflicts.get(p).copied(),
        markers: has_markers(repo, p),
        ..Entry::bare(Group::Conflicts, p)
    });
    let staged = st.staged.iter().map(|c| Entry::change(Group::Staged, c));
    // A conflicted path also shows as unstaged; once is enough.
    let unstaged =
        st.unstaged.iter().filter(|c| !st.conflicted.contains(&c.path)).map(|c| Entry::change(Group::Changes, c));
    let untracked = st.untracked.iter().map(|p| Entry::bare(Group::Untracked, p));
    conflicted.chain(staged).chain(unstaged).chain(untracked).collect()
}

struct App {
    /// Opened again on reload, so the index is read fresh.
    repo: gix::Repository,
    st: Status,
    entries: Vec<Entry>,
    selected: usize,
    /// First display row of the list (headers and gaps count), and which entry each
    /// shown row is, for clicks.
    scroll: usize,
    list_area: Rect,
    shown: Vec<Option<usize>>,
    /// The diff on the right, kept until the cursor moves or the files change.
    diff: Option<View>,
    focus: Focus,
    /// The hunk worked on while the diff has focus.
    hunk: usize,
    /// Lines picked inside that hunk (`v`).
    pick: Option<Pick>,
    detail_scroll: Scroll,
    detail_area: Rect,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
    confirm: Option<(Confirm, Effect)>,
    /// The open commit box, and what was typed in it last time it closed.
    commit: Option<CommitBox>,
    draft: Option<String>,
    repo_name: String,
}

pub fn run(repo: gix::Repository) -> Result<()> {
    app::run(&mut start(repo)?)
}

/// The screen as it opens, before the first draw.
fn start(repo: gix::Repository) -> Result<App> {
    let mut app = App {
        repo_name: crate::git::repo_name(&repo),
        repo,
        st: Status::default(),
        entries: Vec::new(),
        selected: 0,
        scroll: 0,
        list_area: Rect::default(),
        shown: Vec::new(),
        diff: None,
        focus: Focus::Files,
        hunk: 0,
        pick: None,
        detail_scroll: Scroll::default(),
        detail_area: Rect::default(),
        toast: None,
        palette: None,
        confirm: None,
        commit: None,
        draft: None,
    };
    app.reload()?;
    // Start on the first file not staged yet: that's usually the next thing to look at.
    app.selected = app.entries.iter().position(|e| e.group != Group::Staged).unwrap_or(0);
    Ok(app)
}

impl Screen for App {
    type Out = ();
    type Effect = Effect;

    fn draw(&mut self, f: &mut Frame) {
        draw(f, self)
    }
    fn key(&mut self, k: KeyEvent) -> Flow<(), Effect> {
        self.on_key(k)
    }
    fn mouse(&mut self, m: MouseEvent) {
        self.on_mouse(m)
    }
    fn apply(&mut self, e: Effect) -> Result<Flow<(), Effect>> {
        match e {
            Effect::Reload => self.reload()?,
            Effect::File { op, path, msg } => {
                let paths = std::slice::from_ref(&path);
                let r = match op {
                    FileOp::Add => stage::add(&self.repo, paths),
                    FileOp::Unstage => stage::unstage(&self.repo, paths),
                    FileOp::Discard => stage::discard(&self.repo, paths),
                    FileOp::DeleteUntracked => stage::delete_untracked(&self.repo, paths),
                    FileOp::Remove => stage::remove(&self.repo, paths),
                };
                self.done(r, msg)?;
            }
            Effect::StageAll => {
                let r = stage::add_all(&self.repo);
                self.done(r, "Staged everything".into())?;
            }
            Effect::UnstageAll => {
                let r = stage::unstage_all(&self.repo);
                self.done(r, "Unstaged everything".into())?;
            }
            Effect::Hunk { path, hunk, action, msg } => {
                let r = stage::apply_hunk(&self.repo, &path, &hunk, action);
                self.after_hunk(r, msg)?;
            }
            Effect::Copy(path) => {
                crate::clipboard::copy(&path);
                self.toast = Some(Toast::new(format!("Copied {path}"), theme::ACCENT));
            }
            Effect::OpenCommit { amend } => self.open_commit_box(amend),
            Effect::Commit { message, amend, edit } => return self.commit_now(message, amend, edit),
            Effect::CommitOnTerminal { message, amend, edit } => self.commit_on_terminal(&message, amend, edit)?,
            Effect::Edit(path) => return Ok(self.edit(path)),
            Effect::EditOnTerminal(path) => self.edit_on_terminal(&path)?,
        }
        Ok(Flow::Stay)
    }
    fn in_terminal(e: &Effect) -> bool {
        matches!(e, Effect::CommitOnTerminal { .. } | Effect::EditOnTerminal(_))
    }
    fn tick(&mut self) -> Result<()> {
        self.load_view();
        Ok(())
    }
    fn toast(&mut self) -> &mut Option<Toast> {
        &mut self.toast
    }
    fn typing(&self) -> bool {
        self.commit.is_some()
    }
}

fn commands() -> Vec<Command<Cmd>> {
    vec![
        Command { label: "Stage or unstage file", keys: "space", action: Cmd::Toggle },
        Command { label: "Stage hunk by hunk", keys: "enter", action: Cmd::Hunks },
        Command { label: "Commit what's staged", keys: "c", action: Cmd::Commit },
        Command { label: "Amend the last commit", keys: "C", action: Cmd::Amend },
        Command { label: "Stage everything", keys: "a", action: Cmd::StageAll },
        Command { label: "Unstage everything", keys: "u", action: Cmd::UnstageAll },
        Command { label: "Discard changes to file", keys: "d", action: Cmd::Discard },
        Command { label: "Open file in the editor", keys: "e", action: Cmd::Edit },
        Command { label: "Copy path", keys: "y", action: Cmd::Copy },
        Command { label: "Reload", keys: "ctrl+r", action: Cmd::Reload },
        Command { label: "Quit", keys: "q", action: Cmd::Quit },
    ]
}

impl App {
    fn reload(&mut self) -> Result<()> {
        self.repo = gix::open(self.repo.git_dir())?;
        self.st = crate::git::status::read(&self.repo, true)?;
        let conflicts = if self.st.conflicted.is_empty() { Default::default() } else { stage::conflicts(&self.repo)? };
        self.entries = entries(&self.repo, &self.st, &conflicts);
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        self.diff = None;
        // Entry indexes may point at other files now.
        self.pick = None;
        Ok(())
    }

    /// Reloads after a write, keeping the cursor at the same place in the same group,
    /// so space, space, space walks down a list.
    fn reload_at(&mut self, group: Group, pos: usize) -> Result<()> {
        self.reload()?;
        self.put_at(group, pos);
        Ok(())
    }

    /// The cursor on the `pos`-th file of `group`, or as near as there is.
    fn put_at(&mut self, group: Group, pos: usize) {
        let in_group: Vec<usize> = (0..self.entries.len()).filter(|&i| self.entries[i].group == group).collect();
        self.selected = match in_group.get(pos.min(in_group.len().saturating_sub(1))) {
            Some(&i) => i,
            // That group is empty now: the next group's first file, or the last file.
            None => self.entries.iter().position(|e| e.group > group).unwrap_or(self.entries.len().saturating_sub(1)),
        };
        self.hunk = 0;
        self.detail_scroll.reset();
    }

    fn current(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    /// The cursor's group, and its place in it.
    fn place(&self) -> Option<(Group, usize)> {
        let e = self.current()?;
        let pos = self.entries[..self.selected].iter().filter(|o| o.group == e.group).count();
        Some((e.group, pos))
    }

    fn step(&mut self, n: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        let to = (self.selected as isize + n).clamp(0, last) as usize;
        if to != self.selected {
            self.selected = to;
            self.hunk = 0;
            self.detail_scroll.reset();
        }
    }

    /// Shows how a write went, then reloads in place.
    fn done(&mut self, ran: Ran, msg: String) -> Result<()> {
        self.toast = Some(Toast::ran(&ran, msg));
        match self.place() {
            Some((g, pos)) => self.reload_at(g, pos),
            None => self.reload(),
        }
    }

    fn toggle(&mut self) -> Option<Effect> {
        let e = self.current()?;
        let path = e.path.clone();
        match e.group {
            Group::Staged => Some(Effect::File { op: FileOp::Unstage, msg: format!("Unstaged {path}"), path }),
            Group::Conflicts => self.resolve(),
            Group::Changes | Group::Untracked => {
                Some(Effect::File { op: FileOp::Add, msg: format!("Staged {path}"), path })
            }
        }
    }

    fn ask_discard(&mut self) {
        let Some(e) = self.current() else { return };
        let path = e.path.clone();
        let (confirm, ask) = match e.group {
            Group::Staged => {
                self.toast = Some(Toast::new("That's staged: space unstages it first", theme::YELLOW));
                return;
            }
            Group::Conflicts => return self.ask_remove(),
            Group::Untracked => (
                Confirm {
                    title: format!("Delete {path}?"),
                    body: if e.is_folder() {
                        "The folder and every untracked file in it are deleted. git never had them, so they can't be brought back.".into()
                    } else {
                        "git never had this file, so it can't be brought back.".into()
                    },
                    yes: "Delete".into(),
                    no: "Cancel".into(),
                    danger: true,
                },
                Effect::File { op: FileOp::DeleteUntracked, msg: format!("Deleted {path}"), path },
            ),
            Group::Changes => {
                let lines = |n: u32, what: &str| match n {
                    0 => None,
                    1 => Some(format!("1 {what} line")),
                    n => Some(format!("{n} {what} lines")),
                };
                let parts: Vec<String> = match e.size {
                    Some(Size::Lines { added, removed }) => {
                        [lines(added, "added"), lines(removed, "removed")].into_iter().flatten().collect()
                    }
                    _ => Vec::new(),
                };
                let lost = if parts.is_empty() {
                    "Its unstaged changes will be lost.".to_string()
                } else {
                    format!("{} will be lost.", parts.join(" and "))
                };
                let staged = self.entries.iter().any(|o| o.group == Group::Staged && o.path == path);
                let keeps = if staged { "\nThe staged part stays." } else { "" };
                (
                    Confirm {
                        title: format!("Discard changes to {path}?"),
                        body: format!("{lost}{keeps}"),
                        yes: "Discard".into(),
                        no: "Cancel".into(),
                        danger: true,
                    },
                    Effect::File { op: FileOp::Discard, msg: format!("Discarded changes to {path}"), path },
                )
            }
        };
        self.confirm = Some((confirm, ask));
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if self.palette.is_some() || self.confirm.is_some() || self.commit.is_some() {
            return;
        }
        let in_details = list::inside(self.detail_area, &m);
        match m.kind {
            MouseEventKind::ScrollDown if in_details => self.detail_scroll.wheel(true),
            MouseEventKind::ScrollUp if in_details => self.detail_scroll.wheel(false),
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::ScrollUp => self.step(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(row) = list::row_at(self.list_area, 0, &m)
                    && let Some(Some(i)) = self.shown.get(row)
                {
                    self.selected = *i;
                    self.focus = Focus::Files;
                    self.pick = None;
                    self.detail_scroll.reset();
                } else if in_details && self.view().has_hunks() {
                    // A click in the diff works on the hunk under it.
                    let line =
                        self.detail_scroll.top() as usize + m.row.saturating_sub(self.detail_area.y + 1) as usize;
                    if let Some(h) = self.view().hunk_at(line) {
                        self.focus = Focus::Hunks;
                        self.hunk = h;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Does the working tree copy still have `<<<<<<<` lines?
fn has_markers(repo: &gix::Repository, path: &str) -> bool {
    let Some(dir) = repo.workdir() else { return false };
    std::fs::read(dir.join(path)).map(|b| b.split(|&c| c == b'\n').any(|l| l.starts_with(b"<<<<<<< "))).unwrap_or(false)
}
