//! Coloured diffs for details panels (`gg log`, `gg stash`). The file lists above them
//! are in `changes`.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use super::changes::kind_mark;
use crate::git::diff::PatchLine;
use crate::git::tree::{self, FileStat};
use crate::tui::theme;

/// Stop drawing diffs after this many lines; huge commits would make the panel slow.
const MAX_DIFF_LINES: usize = 4000;

/// A header bar per file and its diff, added lines on green and removed on red.
pub fn patches(repo: &gix::Repository, files: &[FileStat]) -> Vec<Line<'static>> {
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
        let Some(lines) = tree::file_patch(repo, f) else {
            out.push(Line::styled("   binary or very large file", theme::faint()));
            continue;
        };
        shown += lines.len();
        out.extend(lines.into_iter().map(line));
    }
    out
}

/// One diff line: line number, `+`/`-`, text; added lines on green, removed on red.
pub fn line(l: PatchLine) -> Line<'static> {
    let added_bg = Color::Rgb(22, 44, 30);
    let removed_bg = Color::Rgb(52, 24, 27);
    match l {
        PatchLine::Hunk { new_start, .. } => Line::styled(format!("   ⋯ line {new_start}"), theme::faint()),
        PatchLine::Context { new, text, .. } => {
            Line::from(vec![Span::styled(format!("{new:>5}  "), theme::faint()), Span::styled(text, theme::muted())])
        }
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
    }
}
