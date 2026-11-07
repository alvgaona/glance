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

/// Diff lines, the lines each file's header and each hunk start on, and the columns
/// of each line that stay put when scrolled sideways (see [`Scroll::view`]).
///
/// [`Scroll::view`]: crate::tui::kit::Scroll::view
#[derive(Default)]
pub struct Patches {
    pub lines: Vec<Line<'static>>,
    pub files: Vec<usize>,
    pub hunks: Vec<usize>,
    pub pins: Vec<u16>,
}

impl Patches {
    /// Put `lines` above: every position moves down.
    pub fn below(mut self, mut lines: Vec<Line<'static>>) -> Self {
        let n = lines.len();
        self.files.iter_mut().chain(&mut self.hunks).for_each(|i| *i += n);
        self.pins.splice(0..0, std::iter::repeat_n(0, n));
        lines.append(&mut self.lines);
        self.lines = lines;
        self
    }

    /// `next` after these.
    pub fn then(mut self, mut next: Patches) -> Self {
        let n = self.lines.len();
        self.files.extend(next.files.iter().map(|i| i + n));
        self.hunks.extend(next.hunks.iter().map(|i| i + n));
        self.pins.resize(n, 0);
        self.pins.append(&mut next.pins);
        self.lines.append(&mut next.lines);
        self
    }
}

/// Columns of line number and `+`/`-` in front of each diff line.
pub const GUTTER: u16 = 7;

/// The columns of `l` that stay put when scrolled sideways: its gutter, or all of a
/// hunk's header.
pub fn pin(l: &PatchLine) -> u16 {
    match l {
        PatchLine::Hunk { .. } => u16::MAX,
        _ => GUTTER,
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
        out.pins.push(0);
        out.files.push(out.lines.len());
        out.pins.push(u16::MAX);
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
            out.pins.push(0);
            continue;
        }
        let Some(lines) = tree::file_patch(repo, f) else {
            out.lines.push(Line::styled("   binary or very large file", theme::faint()));
            out.pins.push(0);
            continue;
        };
        shown += lines.len();
        for l in lines {
            if matches!(l, PatchLine::Hunk { .. }) {
                out.hunks.push(out.lines.len());
            }
            out.pins.push(pin(&l));
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
    // shortcut: nothing pinned, delta draws its own line numbers; goes with delta.
    Some(Patches { lines: out, files: at(drawn.files), hunks: at(drawn.hunks), pins: Vec::new() })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_move_with_what_goes_above_and_before() {
        let p = |n: usize, pin: u16| Patches {
            lines: vec![Line::raw(""); n],
            files: vec![0],
            hunks: vec![1],
            pins: vec![pin; n],
        };
        let joined = p(2, 7).then(p(3, 9)).below(vec![Line::raw("top")]);
        assert_eq!((joined.files, joined.hunks), (vec![1, 3], vec![2, 4]));
        assert_eq!(joined.pins, [0, 7, 7, 9, 9, 9]);
        // Without pins of its own (delta's), the next one's still line up.
        let joined = Patches { lines: vec![Line::raw("")], ..Default::default() }.then(p(1, 7));
        assert_eq!(joined.pins, [0, 7]);
    }
}
