//! Lists of changed files: `M  src/app.rs  +5 −1` rows under a `staged 3   +12 −4`
//! header. Used for a commit's files (`gg log`, `gg stash`) and a working tree's
//! state (`gg wt`, `gg add`).

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::git::diff::Size;
use crate::git::status::{ChangeKind, FileChange, Status};
use crate::git::tree::FileStat;
use crate::tui::theme;

/// The letter for a change and its colour.
pub fn kind_mark(k: ChangeKind) -> (&'static str, Color) {
    match k {
        ChangeKind::Added => ("A", theme::GREEN),
        ChangeKind::Deleted => ("D", theme::RED),
        ChangeKind::Renamed | ChangeKind::Copied => ("R", theme::BLUE),
        ChangeKind::Modified | ChangeKind::TypeChange => ("M", theme::YELLOW),
    }
}

/// One file in a list.
pub struct Item<'a> {
    pub mark: (&'static str, Color),
    pub path: &'a str,
    /// The old path of a rename or copy.
    pub from: Option<&'a str>,
    pub size: Option<Size>,
}

impl<'a> Item<'a> {
    pub fn change(c: &'a FileChange) -> Self {
        Item { mark: kind_mark(c.kind), path: &c.path, from: c.from.as_deref(), size: c.size }
    }

    pub fn stat(f: &'a FileStat) -> Self {
        Item { mark: kind_mark(f.kind), path: &f.path, from: f.from.as_deref(), size: f.size }
    }

    pub fn untracked(path: &'a str) -> Self {
        Item { mark: ("?", theme::MUTED), path, from: None, size: None }
    }

    pub fn conflict(path: &'a str) -> Self {
        Item { mark: ("U", theme::RED), path, from: None, size: None }
    }
}

/// `+5 −1`, `+5`, `−1`, or `binary`; nothing when unknown or no lines changed.
pub fn counts(size: Option<Size>) -> Vec<Span<'static>> {
    match size {
        Some(Size::Lines { added, removed }) => {
            let mut out = Vec::new();
            if added > 0 {
                out.push(Span::styled(format!("+{added}"), Style::new().fg(theme::GREEN)));
            }
            if removed > 0 {
                if added > 0 {
                    out.push(Span::raw(" "));
                }
                out.push(Span::styled(format!("−{removed}"), Style::new().fg(theme::RED)));
            }
            out
        }
        Some(Size::Binary) => vec![Span::styled("binary", theme::faint())],
        None => Vec::new(),
    }
}

/// `label   +12 −4`, the totals of the files that have line counts.
pub fn header(label: String, items: &[Item]) -> Line<'static> {
    let mut total = None;
    for i in items {
        if let Some(Size::Lines { added, removed }) = i.size {
            let (a, r) = total.unwrap_or((0, 0));
            total = Some((a + added, r + removed));
        }
    }
    let mut spans = vec![Span::styled(label, theme::faint())];
    let c = counts(total.map(|(added, removed)| Size::Lines { added, removed }));
    if !c.is_empty() {
        spans.push(Span::raw("   "));
        spans.extend(c);
    }
    Line::from(spans)
}

/// `M  src/app.rs  +5 −1`; a rename reads `old → new`.
pub fn row(i: &Item) -> Line<'static> {
    let (mark, color) = i.mark;
    let name = match i.from {
        Some(from) if from != i.path => format!("{from} → {}", i.path),
        _ => i.path.to_string(),
    };
    let mut spans = vec![Span::styled(format!("{mark}  "), Style::new().fg(color)), Span::styled(name, theme::text())];
    let c = counts(i.size);
    if !c.is_empty() {
        spans.push(Span::raw("  "));
        spans.extend(c);
    }
    Line::from(spans)
}

/// A header and its rows.
pub fn group(label: String, items: &[Item]) -> Vec<Line<'static>> {
    std::iter::once(header(label, items)).chain(items.iter().map(row)).collect()
}

/// A commit's files: `3 files   +24 −5`. `noun` replaces "files" (e.g. "untracked").
pub fn files(list: &[FileStat], noun: Option<&str>) -> Vec<Line<'static>> {
    let n = list.len();
    let noun = noun.map(str::to_string).unwrap_or_else(|| if n == 1 { "file".into() } else { "files".into() });
    let items: Vec<Item> = list.iter().map(Item::stat).collect();
    group(format!("{n} {noun}"), &items)
}

/// A working tree's uncommitted files: conflicts, staged, changes, untracked, each
/// followed by a blank line. Empty groups are left out.
pub fn status(st: &Status) -> Vec<Line<'static>> {
    let groups: [(&str, Vec<Item>); 4] = [
        ("conflicts", st.conflicted.iter().map(|p| Item::conflict(p)).collect()),
        ("staged", st.staged.iter().map(Item::change).collect()),
        ("changes", st.unstaged.iter().map(Item::change).collect()),
        ("untracked", st.untracked.iter().map(|p| Item::untracked(p)).collect()),
    ];
    let mut out = Vec::new();
    for (title, items) in groups {
        if items.is_empty() {
            continue;
        }
        out.extend(group(format!("{title} {}", items.len()), &items));
        out.push(Line::raw(""));
    }
    out
}
