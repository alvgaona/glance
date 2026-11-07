//! Coloured diffs for details panels (`gg log`, `gg stash`). The file lists above them
//! are in `changes`.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use super::changes::kind_mark;
use std::ops::Range;

use crate::git::diff::{self, PatchLine};
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
        for (i, l) in lines.iter().enumerate() {
            if matches!(l, PatchLine::Hunk { .. }) {
                out.hunks.push(out.lines.len() + i);
            }
        }
        out.pins.extend(lines.iter().map(pin));
        out.lines.extend(self::lines(&lines));
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

/// A diff side by side: the old half and the new half of each row, the columns of each
/// row kept in place when scrolled sideways, and the rows hunks start on.
#[derive(Default)]
pub struct Halves {
    pub old: Vec<Line<'static>>,
    pub new: Vec<Line<'static>>,
    pub pins: Vec<u16>,
    pub hunks: Vec<usize>,
}

impl Halves {
    fn row(&mut self, old: Option<Line<'static>>, new: Option<Line<'static>>, pin: u16) {
        self.old.push(old.unwrap_or_default());
        self.new.push(new.unwrap_or_default());
        self.pins.push(pin);
    }

    /// The removed lines next to the added ones that replace them, then the rest of
    /// whichever is longer against blank rows.
    fn pair(&mut self, removed: &mut Vec<Line<'static>>, added: &mut Vec<Line<'static>>) {
        let n = removed.len().max(added.len());
        let (mut r, mut a) = (removed.drain(..), added.drain(..));
        for _ in 0..n {
            self.row(r.next(), a.next(), GUTTER);
        }
    }
}

/// `lines` side by side: removed lines on the left, added ones on the right, context on
/// both with each side's line number.
pub fn side_by_side(lines: Vec<PatchLine>) -> Halves {
    let drawn = self::lines(&lines);
    let mut out = Halves::default();
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    for (l, d) in lines.into_iter().zip(drawn) {
        match l {
            // A removal after additions starts the next change.
            PatchLine::Removed { .. } if !added.is_empty() => {
                out.pair(&mut removed, &mut added);
                removed.push(d);
            }
            PatchLine::Removed { .. } => removed.push(d),
            PatchLine::Added { .. } => added.push(d),
            PatchLine::Hunk { .. } => {
                out.pair(&mut removed, &mut added);
                out.hunks.push(out.old.len());
                out.row(Some(d), None, u16::MAX);
            }
            PatchLine::Context { old, text, .. } => {
                out.pair(&mut removed, &mut added);
                let left = line(PatchLine::Context { old, new: old, text }, &[]);
                out.row(Some(left), Some(d), GUTTER);
            }
        }
    }
    out.pair(&mut removed, &mut added);
    out
}

/// Each diff line drawn, with the words that changed marked: a change's first removed
/// line against its first added line, and so on.
pub fn lines(lines: &[PatchLine]) -> Vec<Line<'static>> {
    let mut marks = vec![Vec::new(); lines.len()];
    for (r, a) in replaced(lines) {
        if let (PatchLine::Removed { text: old, .. }, PatchLine::Added { text: new, .. }) = (&lines[r], &lines[a]) {
            (marks[r], marks[a]) = diff::changed_words(old, new);
        }
    }
    lines.iter().zip(&marks).map(|(l, m)| line(l.clone(), m)).collect()
}

/// Each removed line and the added line taking its place, by position in their change.
fn replaced(lines: &[PatchLine]) -> Vec<(usize, usize)> {
    let is = |i: usize, removed: bool| match lines.get(i) {
        Some(PatchLine::Removed { .. }) => removed,
        Some(PatchLine::Added { .. }) => !removed,
        _ => false,
    };
    let (mut out, mut i) = (Vec::new(), 0);
    while i < lines.len() {
        let first = i;
        while is(i, true) {
            i += 1;
        }
        let added = i;
        while is(i, false) {
            i += 1;
        }
        out.extend((first..added).zip(added..i));
        i = i.max(first + 1);
    }
    out
}

/// One diff line: line number, `+`/`-`, text; added lines on green, removed on red,
/// the bytes in `marks` brighter.
fn line(l: PatchLine, marks: &[Range<usize>]) -> Line<'static> {
    let text = |text: String, bg: Color| {
        let mut out = Vec::new();
        let mut at = 0;
        for m in marks {
            out.push(Span::styled(text[at..m.start].to_string(), theme::text()));
            out.push(Span::styled(text[m.clone()].to_string(), theme::text().bg(bg)));
            at = m.end;
        }
        out.push(Span::styled(text[at..].to_string(), theme::text()));
        out.retain(|s| !s.content.is_empty());
        out
    };
    match l {
        PatchLine::Hunk { new_start, .. } => Line::styled(format!("   ⋯ line {new_start}"), theme::faint()),
        PatchLine::Context { new, text, .. } => {
            Line::from(vec![Span::styled(format!("{new:>5}  "), theme::faint()), Span::styled(text, theme::muted())])
        }
        PatchLine::Added { new, text: t } => {
            let mut spans = vec![Span::styled(format!("{new:>5} +"), Style::new().fg(theme::GREEN))];
            spans.extend(text(t, ADDED_MARK));
            Line::from(spans).style(Style::new().bg(ADDED))
        }
        PatchLine::Removed { old, text: t } => {
            let mut spans = vec![Span::styled(format!("{old:>5} -"), Style::new().fg(theme::RED))];
            spans.extend(text(t, REMOVED_MARK));
            Line::from(spans).style(Style::new().bg(REMOVED))
        }
    }
}

/// Behind added and removed lines, and brighter behind the words that changed in them.
const ADDED: Color = Color::Rgb(22, 44, 30);
const ADDED_MARK: Color = Color::Rgb(38, 92, 54);
const REMOVED: Color = Color::Rgb(52, 24, 27);
const REMOVED_MARK: Color = Color::Rgb(112, 42, 50);

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

    #[test]
    fn side_by_side_pairs_changes() {
        let text = |t: &str| t.to_string();
        let lines = vec![
            PatchLine::Hunk { old_start: 4, new_start: 4 },
            PatchLine::Context { old: 4, new: 4, text: text("a") },
            PatchLine::Removed { old: 5, text: text("b") },
            PatchLine::Removed { old: 6, text: text("c") },
            PatchLine::Added { new: 5, text: text("B") },
            PatchLine::Context { old: 7, new: 6, text: text("d") },
            PatchLine::Added { new: 7, text: text("e") },
        ];
        let h = side_by_side(lines);
        let side = |v: &[Line]| v.iter().map(|l| l.to_string().trim().to_string()).collect::<Vec<_>>();
        assert_eq!(side(&h.old), ["⋯ line 4", "4  a", "5 -b", "6 -c", "7  d", ""]);
        assert_eq!(side(&h.new), ["", "4  a", "5 +B", "", "6  d", "7 +e"]);
        assert_eq!(h.hunks, [0]);
        assert_eq!(h.pins, [u16::MAX, 7, 7, 7, 7, 7]);
    }

    #[test]
    fn marks_the_words_that_changed() {
        let lines = [
            PatchLine::Removed { old: 1, text: "let n = 2;".into() },
            PatchLine::Added { new: 1, text: "let n = 3;".into() },
            PatchLine::Added { new: 2, text: "more".into() },
        ];
        let marked = |l: &Line, bg: Color| {
            l.spans.iter().filter(|s| s.style.bg == Some(bg)).map(|s| s.content.to_string()).collect::<Vec<_>>()
        };
        let drawn = super::lines(&lines);
        assert_eq!(marked(&drawn[0], REMOVED_MARK), ["2"]);
        assert_eq!(marked(&drawn[1], ADDED_MARK), ["3"]);
        // Nothing it replaces: added whole, nothing marked.
        assert!(marked(&drawn[2], ADDED_MARK).is_empty());
        assert_eq!(drawn[1].to_string(), "    1 +let n = 3;");
    }
}
