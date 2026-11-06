//! The changes pane of `gg stash`: choose files and a mode, then `git stash push`.

use std::collections::{BTreeMap, BTreeSet};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::fmt::change_letter;
use crate::git::diff::Size;
pub use crate::git::stash::Mode;
use crate::git::stash::Push;
use crate::git::status::{ChangeKind, Status};
use crate::tui::panels::changes;
use crate::tui::{kit, theme};

const STAGED_ONLY_ALL: &str = "This mode takes every file: git can't pick single files here. Press e to pick.";

fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Everything => "everything",
        Mode::Staged => "staged only",
        Mode::Unstaged => "unstaged only",
    }
}

/// One path in the working tree, with its staged and unstaged state (like `git status -s`).
pub struct File {
    pub path: String,
    pub staged: Option<ChangeKind>,
    pub unstaged: Option<ChangeKind>,
    pub untracked: bool,
    added: u32,
    removed: u32,
    binary: bool,
}

/// What the user picked: the push to run once the message is known, and how many files.
pub struct Picked {
    pub push: Push,
    pub count: usize,
}

impl Picked {
    pub fn describe(&self) -> String {
        let files = if self.count == 1 { "1 file".to_string() } else { format!("{} files", self.count) };
        format!("{files}, {}", mode_label(self.push.mode))
    }
}

pub struct Picker {
    files: Vec<File>,
    pub mode: Mode,
    pub untracked: bool,
    /// Files the user unticked.
    off: BTreeSet<String>,
    /// Position among the files the mode covers.
    cursor: usize,
    /// A one-off hint after a key that can't do anything here.
    notice: Option<String>,
    /// First file row shown, and where the rows were drawn (for the mouse).
    scroll: usize,
    rows: Rect,
}

impl Picker {
    pub fn new(st: &Status) -> Self {
        Self {
            files: read_files(st),
            mode: Mode::Everything,
            untracked: true,
            off: BTreeSet::new(),
            cursor: 0,
            notice: None,
            scroll: 0,
            rows: Rect::default(),
        }
    }

    /// New working tree state; keeps the mode, and unticked files that are still there.
    pub fn update(&mut self, st: &Status) {
        self.files = read_files(st);
        let paths: BTreeSet<&str> = self.files.iter().map(|f| f.path.as_str()).collect();
        self.off.retain(|p| paths.contains(p.as_str()));
        self.cursor = self.cursor.min(self.covered().len().saturating_sub(1));
    }

    /// Ticks every file again, after a stash.
    pub fn reset_ticks(&mut self) {
        self.off.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// One row per path; staged and unstaged changes to the same path merge.
fn read_files(st: &Status) -> Vec<File> {
    let blank = |path: &str| File {
        path: path.to_string(),
        staged: None,
        unstaged: None,
        untracked: false,
        added: 0,
        removed: 0,
        binary: false,
    };
    let mut map: BTreeMap<String, File> = BTreeMap::new();
    // Staged and unstaged changes to the same path merge into one row.
    for (list, staged) in [(&st.staged, true), (&st.unstaged, false)] {
        for c in list {
            let f = map.entry(c.path.clone()).or_insert_with(|| blank(&c.path));
            if staged {
                f.staged = Some(c.kind);
            } else {
                f.unstaged = Some(c.kind);
            }
            match c.size {
                Some(Size::Lines { added, removed }) => {
                    f.added += added;
                    f.removed += removed;
                }
                Some(Size::Binary) => f.binary = true,
                None => {}
            }
        }
    }
    for path in &st.untracked {
        map.entry(path.clone()).or_insert_with(|| blank(path)).untracked = true;
    }
    map.into_values().collect()
}

impl Picker {
    /// Does the mode take this file at all?
    fn covers(&self, f: &File) -> bool {
        match self.mode {
            Mode::Everything => f.staged.is_some() || f.unstaged.is_some() || (f.untracked && self.untracked),
            Mode::Staged => f.staged.is_some(),
            Mode::Unstaged => f.unstaged.is_some() || (f.untracked && self.untracked),
        }
    }

    fn covered(&self) -> Vec<usize> {
        (0..self.files.len()).filter(|&i| self.covers(&self.files[i])).collect()
    }

    /// `--staged` and `--keep-index` ignore file lists, so those modes take every file they cover.
    fn tickable(&self) -> bool {
        self.mode == Mode::Everything
    }

    fn is_on(&self, f: &File) -> bool {
        !self.tickable() || !self.off.contains(&f.path)
    }

    fn picked(&self) -> Picked {
        let covered = self.covered();
        let on: Vec<String> =
            covered.iter().map(|&i| &self.files[i]).filter(|f| self.is_on(f)).map(|f| f.path.clone()).collect();
        let count = on.len();
        let paths = if count == covered.len() { None } else { Some(on) };
        Picked { push: Push { mode: self.mode, untracked: self.untracked, paths }, count }
    }

    /// Why git would refuse, if it would.
    fn blocked(&self) -> Option<String> {
        if self.mode != Mode::Staged {
            return None;
        }
        // `git stash push --staged` fails (and leaves a stray stash) on these.
        let both: Vec<&str> =
            self.files.iter().filter(|f| f.staged.is_some() && f.unstaged.is_some()).map(|f| f.path.as_str()).collect();
        if both.is_empty() {
            return None;
        }
        let list = if both.len() > 3 {
            format!("{} and {} more", both[..3].join(", "), both.len() - 3)
        } else {
            both.join(", ")
        };
        Some(format!(
            "git can't stash only the staged part of a file that also has unstaged changes: {list}. Stage the rest or undo it first."
        ))
    }

    /// The muted lines under the list: what this will do, and git's quirks.
    fn explain(&self, picked: &Picked) -> String {
        let mut s = match self.mode {
            Mode::Everything => "Saves the ticked files and resets them to the last commit.".to_string(),
            Mode::Staged => "Saves every staged change. Everything else stays as it is.".to_string(),
            Mode::Unstaged => "Saves every unstaged change. Staged changes stay in place.".to_string(),
        };
        // git always records the whole index, even with a file list or --keep-index.
        let left_staged = match (&picked.push.paths, self.mode) {
            (_, Mode::Unstaged) => self.files.iter().any(|f| f.staged.is_some()),
            (Some(on), _) => self.files.iter().any(|f| f.staged.is_some() && !on.contains(&f.path)),
            _ => false,
        };
        if left_staged {
            s += "\nStaged changes left here are copied into the stash too: git always saves the whole index.";
        }
        if let Some(n) = &self.notice {
            s += &format!("\n{n}");
        }
        s
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.cursor = 0;
    }

    pub fn toggle_untracked(&mut self) {
        self.untracked = !self.untracked;
        self.cursor = self.cursor.min(self.covered().len().saturating_sub(1));
    }

    /// Ticks every file, or unticks them all when all are ticked.
    pub fn toggle_all(&mut self) {
        if !self.tickable() {
            self.notice = Some(STAGED_ONLY_ALL.into());
            return;
        }
        let covered = self.covered();
        let all_on = covered.iter().all(|&i| !self.off.contains(&self.files[i].path));
        for i in covered {
            let p = self.files[i].path.clone();
            if all_on {
                self.off.insert(p);
            } else {
                self.off.remove(&p);
            }
        }
    }

    /// Keys for the pane; returns what to stash when enter is pressed and git would take it.
    pub fn key(&mut self, k: KeyEvent) -> Option<Picked> {
        let last = self.covered().len().saturating_sub(1);
        self.notice = None;
        match k.code {
            KeyCode::Enter => {
                let p = self.picked();
                if self.blocked().is_none() && p.count > 0 {
                    return Some(p);
                }
            }
            KeyCode::Char(' ') if !self.tickable() => self.notice = Some(STAGED_ONLY_ALL.into()),
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Home | KeyCode::Char('g') => self.cursor = 0,
            KeyCode::End | KeyCode::Char('G') => self.cursor = last,
            KeyCode::Char(' ') => {
                if let Some(&i) = self.covered().get(self.cursor) {
                    let p = self.files[i].path.clone();
                    if !self.off.remove(&p) {
                        self.off.insert(p);
                    }
                    self.cursor = (self.cursor + 1).min(last);
                }
            }
            KeyCode::Char('a') => self.toggle_all(),
            KeyCode::Char('e') => self.set_mode(Mode::Everything),
            KeyCode::Char('s') => self.set_mode(Mode::Staged),
            KeyCode::Char('u') => self.set_mode(Mode::Unstaged),
            KeyCode::Char('t') => self.toggle_untracked(),
            _ => {}
        }
        None
    }

    /// Footer keys while this pane has focus, most important first.
    pub fn hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = vec![("enter", "stash")];
        if self.tickable() {
            out.extend([("space", "tick"), ("a", "all")]);
        }
        out.extend([("e/s/u", "mode"), ("t", "untracked")]);
        out
    }

    /// Mouse click on a screen row: puts the cursor on that file.
    pub fn click(&mut self, row: u16) {
        if row < self.rows.y || row >= self.rows.bottom() {
            return;
        }
        let i = self.scroll + (row - self.rows.y) as usize;
        if i < self.covered().len() {
            self.cursor = i;
        }
    }

    pub fn step(&mut self, n: isize) {
        let last = self.covered().len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + n).clamp(0, last) as usize;
    }

    /// The note under the files, wrapped; red when git would refuse.
    fn note(&self, width: u16) -> (Vec<String>, bool) {
        let blocked = self.blocked();
        let red = blocked.is_some();
        let text = blocked.unwrap_or_else(|| self.explain(&self.picked()));
        (text.lines().flat_map(|l| kit::wrap(l, width as usize)).collect(), red)
    }

    fn others_rows(&self) -> usize {
        let n = self.files.len() - self.covered().len();
        if n == 0 { 0 } else { 2 + n.min(3) + usize::from(n > 3) }
    }

    /// Rows the pane needs to show everything at this width (panel padding included).
    pub fn height(&self, width: u16) -> u16 {
        if self.files.is_empty() {
            return 5;
        }
        let inner_w = width.saturating_sub(4);
        // Title, gap, tabs, gap; files; others; gap, note; padding.
        (4 + self.covered().len().max(1) + self.others_rows() + 1 + self.note(inner_w).0.len() + 2) as u16
    }

    /// Columns the pane needs: its widest file row, or the mode tabs; padding included.
    pub fn width(&self) -> u16 {
        let name_w = self.files.iter().map(|f| f.path.chars().count()).max().unwrap_or(0);
        let rows = self.files.iter().map(|f| self.row(f, Some(true), false, name_w, 0).width()).max().unwrap_or(0);
        // " e everything ", " s staged ", " u unstaged " and " t untracked off ", with gaps.
        let tabs = 57;
        (rows.max(tabs) + 4).min(u16::MAX as usize) as u16
    }

    /// The pane: title, mode tabs, the files, and a note on what enter does.
    pub fn draw(&mut self, f: &mut Frame, area: Rect, focused: bool) {
        let block = kit::panel(theme::PANEL);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let title = if focused { Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD) } else { theme::bold() };
        let picked = self.picked();
        let summary = if self.files.is_empty() { "working tree clean".to_string() } else { picked.describe() };
        let mut lines = vec![Line::from(vec![
            Span::styled("changes", title),
            Span::styled(format!("   {summary}"), theme::muted()),
        ])];
        lines.push(Line::raw(""));
        if self.files.is_empty() {
            self.rows = Rect::default();
            lines.push(Line::styled("Nothing to stash.", theme::muted()));
            f.render_widget(Paragraph::new(lines), inner);
            return;
        }

        let covered = self.covered();
        let others: Vec<usize> = (0..self.files.len()).filter(|i| !covered.contains(i)).collect();
        let others_rows = self.others_rows();
        let (note, red) = self.note(inner.width);

        // Mode tabs and the untracked switch.
        let modes =
            [(Mode::Everything, "e", "everything"), (Mode::Staged, "s", "staged"), (Mode::Unstaged, "u", "unstaged")];
        let mut tabs: Vec<Span> = (modes.into_iter())
            .flat_map(|(m, key, label)| {
                let style = if m == self.mode {
                    Style::new().bg(theme::ACCENT).fg(theme::BG)
                } else {
                    Style::new().bg(theme::BORDER).fg(theme::TEXT)
                };
                [Span::styled(format!(" {key} {label} "), style), Span::raw(" ")]
            })
            .collect();
        let untracked_on = self.untracked && self.mode != Mode::Staged;
        tabs.push(Span::raw(" "));
        tabs.push(Span::styled(
            format!(" t untracked {} ", if untracked_on { "on" } else { "off" }),
            Style::new().bg(theme::BORDER).fg(if untracked_on { theme::GREEN } else { theme::MUTED }),
        ));
        lines.push(Line::from(tabs));
        lines.push(Line::raw(""));

        // Rows that fit, scrolled to keep the cursor in view.
        // Title, gap, tabs, gap above the rows; the other files, gap and note below.
        let fixed = 5 + note.len() + others_rows;
        let room = (inner.height as usize).saturating_sub(fixed).max(1);
        self.cursor = self.cursor.min(covered.len().saturating_sub(1));
        self.scroll = kit::list::follow(self.scroll, self.cursor, self.cursor, room);
        self.rows = Rect { y: inner.y + 4, height: room as u16, ..inner };
        let name_w = self.files.iter().map(|f| f.path.chars().count()).max().unwrap_or(0).min(inner.width as usize / 2);
        lines.extend(covered.iter().enumerate().skip(self.scroll).take(room).map(|(n, &i)| {
            let file = &self.files[i];
            self.row(file, Some(self.is_on(file)), focused && n == self.cursor, name_w, inner.width)
        }));
        if covered.is_empty() {
            lines.push(Line::styled("  Nothing for this mode", theme::muted()));
        }

        if !others.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::styled("Stays in the working tree", theme::faint()));
            lines.extend(others.iter().take(3).map(|&i| self.row(&self.files[i], None, false, name_w, inner.width)));
            if others.len() > 3 {
                lines.push(Line::styled(format!("  …and {} more", others.len() - 3), theme::faint()));
            }
        }

        lines.push(Line::raw(""));
        let note_style = if red { Style::new().fg(theme::RED) } else { theme::muted() };
        lines.extend(note.into_iter().map(|l| Line::styled(l, note_style)));
        let lines: Vec<Line> = lines.into_iter().map(|l| kit::truncate(l, inner.width)).collect();
        f.render_widget(Paragraph::new(lines), inner);
    }

    /// `┃ ✓ MM  src/app.rs   +5 −1`. `on` is None for files the mode leaves alone.
    fn row(&self, file: &File, on: Option<bool>, selected: bool, name_w: usize, width: u16) -> Line<'static> {
        let bg = if selected { theme::ELEMENT } else { theme::PANEL };
        let st = |s: Style| s.bg(bg);
        let letter = |k: Option<ChangeKind>, color: Color| match k {
            Some(k) => Span::styled(change_letter(k).to_string(), st(Style::new().fg(color))),
            None => Span::styled(" ", st(Style::new())),
        };
        let mut spans = vec![Span::styled(if selected { "┃ " } else { "  " }, st(Style::new().fg(theme::ACCENT)))];
        spans.push(match on {
            Some(true) => Span::styled("✓ ", st(Style::new().fg(theme::ACCENT))),
            Some(false) => Span::styled("· ", st(Style::new().fg(theme::FAINT))),
            None => Span::styled("  ", st(Style::new())),
        });
        if file.untracked {
            spans.push(Span::styled("??", st(Style::new().fg(theme::MUTED))));
        } else {
            spans.push(letter(file.staged, theme::GREEN));
            spans.push(letter(file.unstaged, theme::YELLOW));
        }
        let text = match on {
            Some(true) => theme::text(),
            _ => theme::muted(),
        };
        let pad = " ".repeat(name_w.saturating_sub(file.path.chars().count()) + 2);
        spans.push(Span::styled(format!("  {}{pad}", file.path), st(text)));
        let size = if file.binary { Size::Binary } else { Size::Lines { added: file.added, removed: file.removed } };
        spans.extend(changes::counts(Some(size)).into_iter().map(|s| s.patch_style(st(Style::new()))));
        let used = Line::from(spans.clone()).width();
        spans.push(Span::styled(" ".repeat((width as usize).saturating_sub(used)), st(Style::new())));
        Line::from(spans)
    }
}
