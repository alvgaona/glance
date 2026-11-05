//! File lists and coloured diffs for details panels (`gg log`, `gg stash`).

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use super::theme;
use crate::core::diff::{PatchLine, Size};
use crate::core::show::{self, FileStat};
use crate::core::status::ChangeKind;

/// Stop drawing diffs after this many lines; huge commits would make the panel slow.
const MAX_DIFF_LINES: usize = 4000;

pub fn kind_mark(k: ChangeKind) -> (&'static str, Color) {
    match k {
        ChangeKind::Added => ("A", theme::GREEN),
        ChangeKind::Deleted => ("D", theme::RED),
        ChangeKind::Renamed | ChangeKind::Copied => ("R", theme::BLUE),
        ChangeKind::Modified | ChangeKind::TypeChange => ("M", theme::YELLOW),
    }
}

/// `3 files   +24 −5` and one row per file. `title` replaces "files" (e.g. "untracked").
pub fn file_list(files: &[FileStat], title: Option<&str>) -> Vec<Line<'static>> {
    let (mut ta, mut tr) = (0, 0);
    for f in files {
        if let Some(Size::Lines { added, removed }) = f.size {
            ta += added;
            tr += removed;
        }
    }
    let n = files.len();
    let noun = title.map(str::to_string).unwrap_or_else(|| if n == 1 { "file".into() } else { "files".into() });
    let mut out = vec![Line::from(vec![
        Span::styled(format!("{n} {noun}   "), theme::faint()),
        Span::styled(format!("+{ta} "), Style::new().fg(theme::GREEN)),
        Span::styled(format!("−{tr}"), Style::new().fg(theme::RED)),
    ])];
    for f in files {
        let (mark, color) = kind_mark(f.kind);
        let mut spans = vec![
            Span::styled(format!("{mark}  "), Style::new().fg(color)),
            Span::styled(f.path.clone(), theme::text()),
        ];
        if let Some(Size::Lines { added, removed }) = f.size {
            spans.push(Span::styled(format!("  +{added} "), Style::new().fg(theme::GREEN)));
            spans.push(Span::styled(format!("−{removed}"), Style::new().fg(theme::RED)));
        }
        out.push(Line::from(spans));
    }
    out
}

/// A header bar per file and its diff, added lines on green and removed on red.
pub fn patches(repo: &gix::Repository, files: &[FileStat]) -> Vec<Line<'static>> {
    let added_bg = Color::Rgb(22, 44, 30);
    let removed_bg = Color::Rgb(52, 24, 27);
    let mut out = Vec::new();
    let mut shown = 0;
    for f in files {
        out.push(Line::raw(""));
        let (mark, color) = kind_mark(f.kind);
        let name = match &f.from {
            Some(from) => format!("{from} → {}", f.path),
            None => f.path.clone(),
        };
        out.push(
            Line::from(vec![
                Span::styled(format!(" {mark} "), Style::new().fg(color)),
                Span::styled(name, theme::bold()),
            ])
            .style(Style::new().bg(theme::ELEMENT)),
        );
        if shown >= MAX_DIFF_LINES {
            out.push(Line::styled("   diff not shown: too many changes", theme::faint()));
            continue;
        }
        let Some(lines) = show::file_patch(repo, f) else {
            out.push(Line::styled("   binary or very large file", theme::faint()));
            continue;
        };
        for l in lines {
            shown += 1;
            out.push(match l {
                PatchLine::Hunk { new_start, .. } => Line::styled(format!("   ⋯ line {new_start}"), theme::faint()),
                PatchLine::Context { new, text, .. } => Line::from(vec![
                    Span::styled(format!("{new:>5}  "), theme::faint()),
                    Span::styled(text, theme::muted()),
                ]),
                PatchLine::Added { new, text } => Line::from(vec![
                    Span::styled(format!("{new:>5} "), Style::new().fg(theme::GREEN)),
                    Span::styled("+", Style::new().fg(theme::GREEN)),
                    Span::styled(text, theme::text()),
                ])
                .style(Style::new().bg(added_bg)),
                PatchLine::Removed { old, text } => Line::from(vec![
                    Span::styled(format!("{old:>5} "), Style::new().fg(theme::RED)),
                    Span::styled("-", Style::new().fg(theme::RED)),
                    Span::styled(text, theme::text()),
                ])
                .style(Style::new().bg(removed_bg)),
            });
        }
    }
    out
}
