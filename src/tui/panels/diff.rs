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

/// Diff lines, and the lines each file's header and each hunk start on.
#[derive(Default)]
pub struct Patches {
    pub lines: Vec<Line<'static>>,
    pub files: Vec<usize>,
    pub hunks: Vec<usize>,
}

impl Patches {
    /// Put `lines` above: every position moves down.
    pub fn below(mut self, mut lines: Vec<Line<'static>>) -> Self {
        let n = lines.len();
        self.files.iter_mut().chain(&mut self.hunks).for_each(|i| *i += n);
        lines.append(&mut self.lines);
        self.lines = lines;
        self
    }
}

/// Each file's diff, `width` columns wide: drawn by delta when it's installed, else a
/// header bar per file and its lines, added on green and removed on red.
pub fn patches(repo: &gix::Repository, files: &[FileStat], width: u16) -> Patches {
    if crate::delta::available()
        && let Some(p) = by_delta(repo, files, width)
    {
        return p;
    }
    let mut out = Patches::default();
    let mut shown = 0;
    for f in files {
        out.lines.push(Line::raw(""));
        out.files.push(out.lines.len());
        let (mark, color) = kind_mark(f.kind);
        let name = match &f.from {
            Some(from) => format!("{from} → {}", f.path),
            None => f.path.clone(),
        };
        out.lines.push(
            Line::from(vec![
                Span::styled(format!(" {mark} "), Style::new().fg(color)),
                Span::styled(name, theme::bold()),
            ])
            .style(Style::new().bg(theme::ELEMENT)),
        );
        if shown >= MAX_DIFF_LINES {
            out.lines.push(Line::styled("   diff not shown: too many changes", theme::faint()));
            continue;
        }
        let Some(lines) = tree::file_patch(repo, f) else {
            out.lines.push(Line::styled("   binary or very large file", theme::faint()));
            continue;
        };
        shown += lines.len();
        for l in lines {
            if matches!(l, PatchLine::Hunk { .. }) {
                out.hunks.push(out.lines.len());
            }
            out.lines.push(line(l));
        }
    }
    out
}

/// All the files in one delta run; `None` if delta failed.
fn by_delta(repo: &gix::Repository, files: &[FileStat], width: u16) -> Option<Patches> {
    let mut text = Vec::new();
    let (mut left, mut lines) = (files.len(), 0);
    for f in files {
        if lines >= MAX_DIFF_LINES {
            break;
        }
        let t = tree::file_text(repo, f);
        lines += t.iter().filter(|&&b| b == b'\n').count();
        text.extend(t);
        left -= 1;
    }
    // The panel is too narrow for side by side.
    let drawn = crate::delta::render(&String::from_utf8_lossy(&text), width, &[("side-by-side", "false")]).ok()?;
    let mut out = vec![Line::raw("")];
    out.extend(crate::tui::kit::ansi::lines(&drawn.text));
    if left > 0 {
        out.push(Line::styled(format!("   {left} more files not shown: too many changes"), theme::faint()));
    }
    // The blank line on top moves everything down one.
    let at = |v: Vec<usize>| v.into_iter().map(|i| i + 1).collect();
    Some(Patches { lines: out, files: at(drawn.files), hunks: at(drawn.hunks) })
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
