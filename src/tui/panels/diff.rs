//! Coloured diffs, for the diff screen and the details panels (`gg log`, `gg stash`).
//! The file lists above them are in `changes`.

use std::ops::Range;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use super::changes::kind_mark;
use super::syntax::Painter;
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
    pub later: Later,
}

impl Patches {
    /// Colours more of the diffs once rows `top..bottom`, those on screen, come near
    /// where their colouring stopped.
    pub fn near(&mut self, top: usize, bottom: usize) {
        self.later.near(&mut self.lines, top, bottom);
    }

    /// Put `lines` above: every position moves down.
    pub fn below(mut self, mut lines: Vec<Line<'static>>) -> Self {
        let n = lines.len();
        self.files.iter_mut().chain(&mut self.hunks).for_each(|i| *i += n);
        self.pins.splice(0..0, std::iter::repeat_n(0, n));
        self.later.0.iter_mut().for_each(|(row, _)| *row += n);
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
        self.later.0.extend(next.later.0.into_iter().map(|(row, l)| (row + n, l)));
        self.lines.append(&mut next.lines);
        self
    }
}

/// Columns in front of each diff line: its line number, if shown, and `+`/`-`.
fn gutter(numbers: bool) -> u16 {
    if numbers { 7 } else { 1 }
}

/// The columns of `l` that stay put when scrolled sideways: its gutter, or all of a
/// hunk's header.
pub fn pin(l: &PatchLine, numbers: bool) -> u16 {
    match l {
        PatchLine::Hunk { .. } => u16::MAX,
        _ => gutter(numbers),
    }
}

/// Each file's diff: a header bar per file and its lines, added on green and removed on
/// red.
pub fn patches(repo: &gix::Repository, files: &[FileStat]) -> Patches {
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
        out.pins.extend(lines.iter().map(|l| pin(l, true)));
        let lazy = Lazy::new(lines, &f.path, true);
        out.lines.extend(lazy.lines());
        out.later.push(out.lines.len() - lazy.patch().len(), lazy);
    }
    out
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
    fn pair(&mut self, removed: &mut Vec<Line<'static>>, added: &mut Vec<Line<'static>>, pin: u16) {
        let n = removed.len().max(added.len());
        let (mut r, mut a) = (removed.drain(..), added.drain(..));
        for _ in 0..n {
            self.row(r.next(), a.next(), pin);
        }
    }
}

/// Rows past the bottom of the screen coloured ahead, once the screen is this near
/// where colouring stopped: a line takes ~40µs, too slow to colour a long diff at once.
const AHEAD: usize = 500;
const NEAR: usize = 200;

/// One file's diff, its code coloured only as far as it has been near the screen.
pub struct Lazy {
    patch: Vec<PatchLine>,
    /// The words that changed in each line: a change's first removed line against its
    /// first added line, and so on.
    marks: Vec<Vec<Range<usize>>>,
    painter: Painter,
    numbers: bool,
}

impl Lazy {
    /// `patch` of the file at `path`, for its language; `numbers`: with line numbers.
    /// Nothing is coloured yet.
    pub fn new(patch: Vec<PatchLine>, path: &str, numbers: bool) -> Self {
        let mut marks = vec![Vec::new(); patch.len()];
        for (r, a) in replaced(&patch) {
            if let (PatchLine::Removed { text: old, .. }, PatchLine::Added { text: new, .. }) = (&patch[r], &patch[a]) {
                (marks[r], marks[a]) = diff::changed_words(old, new);
            }
        }
        Lazy { patch, marks, painter: Painter::new(path), numbers }
    }

    pub fn patch(&self) -> &[PatchLine] {
        &self.patch
    }

    /// Colours ahead once line `i` of the patch, on screen, is near where colouring
    /// stopped. True when it coloured more: draw it again.
    pub fn near(&mut self, i: usize) -> bool {
        let done = self.painter.done();
        // shortcut: colouring runs from the top, so a far jump (G in a 20k-line lockfile)
        // would take a second; those lines stay plain. Colour in the background to lift it.
        if done >= self.patch.len() || i + NEAR < done || i > done + AHEAD {
            return false;
        }
        self.painter.reach(&self.patch, i + AHEAD)
    }

    /// Each line drawn: a line number, `+`/`-`, the code with the changed words marked.
    pub fn lines(&self) -> Vec<Line<'static>> {
        let drawn = self.patch.iter().zip(&self.marks).enumerate();
        drawn.map(|(i, (l, m))| line(l.clone(), m, self.painter.colors(i), self.numbers)).collect()
    }

    /// Side by side: removed lines on the left, added ones on the right, context on both
    /// with each side's line number.
    pub fn halves(&self) -> Halves {
        let numbers = self.numbers;
        let pin = gutter(numbers);
        let mut out = Halves::default();
        let (mut removed, mut added) = (Vec::new(), Vec::new());
        for (l, d) in self.patch.iter().zip(self.lines()) {
            match *l {
                // A removal after additions starts the next change.
                PatchLine::Removed { .. } if !added.is_empty() => {
                    out.pair(&mut removed, &mut added, pin);
                    removed.push(d);
                }
                PatchLine::Removed { .. } => removed.push(d),
                PatchLine::Added { .. } => added.push(d),
                PatchLine::Hunk { .. } => {
                    out.pair(&mut removed, &mut added, pin);
                    out.hunks.push(out.old.len());
                    out.row(Some(d), None, u16::MAX);
                }
                PatchLine::Context { old, .. } => {
                    out.pair(&mut removed, &mut added, pin);
                    // The same line with the old side's number in front.
                    let mut left = d.clone();
                    left.spans[0] = Span::styled(sign_gutter(old, ' ', numbers), theme::faint());
                    out.row(Some(left), Some(d), pin);
                }
            }
        }
        out.pair(&mut removed, &mut added, pin);
        out
    }
}

/// Diffs drawn into a list of lines, each from its row, to colour as they scroll near.
#[derive(Default)]
pub struct Later(Vec<(usize, Lazy)>);

impl Later {
    pub const NONE: Later = Later(Vec::new());

    /// `lazy`, drawn from `row` on.
    pub fn push(&mut self, row: usize, lazy: Lazy) {
        self.0.push((row, lazy));
    }

    /// Colours more of the diffs near rows `top..bottom` of `lines`, those on screen, and
    /// draws them again there.
    pub fn near(&mut self, lines: &mut [Line<'static>], top: usize, bottom: usize) {
        for (row, l) in &mut self.0 {
            // Above the screen, or far below: later.
            if *row + l.patch.len() <= top || *row > bottom + AHEAD {
                continue;
            }
            if l.near(bottom.saturating_sub(*row)) {
                for (at, d) in lines[*row..].iter_mut().zip(l.lines()) {
                    *at = d;
                }
            }
        }
    }
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

fn sign_gutter(n: u32, sign: char, numbers: bool) -> String {
    if numbers { format!("{n:>5} {sign}") } else { sign.to_string() }
}

/// One diff line: line number, `+`/`-`, text in `colors`; added lines on green, removed
/// on red, the bytes in `marks` brighter.
fn line(l: PatchLine, marks: &[Range<usize>], colors: &[(Range<usize>, Color)], numbers: bool) -> Line<'static> {
    let gutter = |n: u32, sign: char| sign_gutter(n, sign, numbers);
    let text = |text: String, bg: Color| {
        // Cut wherever a colour or a mark starts or ends.
        let edges = colors.iter().map(|(r, _)| r).chain(marks).flat_map(|r| [r.start, r.end]);
        let mut cuts: Vec<usize> = edges.chain([0, text.len()]).filter(|&c| c <= text.len()).collect();
        cuts.sort_unstable();
        cuts.dedup();
        let within = |r: &Range<usize>, at: usize| r.start <= at && at < r.end;
        cuts.windows(2)
            .map(|w| {
                let mut style = theme::text();
                if let Some((_, c)) = colors.iter().find(|(r, _)| within(r, w[0])) {
                    style = style.fg(*c);
                }
                if marks.iter().any(|m| within(m, w[0])) {
                    style = style.bg(bg);
                }
                Span::styled(text[w[0]..w[1]].to_string(), style)
            })
            .collect::<Vec<_>>()
    };
    match l {
        PatchLine::Hunk { new_start, .. } => Line::styled(format!("   ⋯ line {new_start}"), theme::faint()),
        // Uncoloured code stays dimmer than the changes around it.
        PatchLine::Context { new, text, .. } if colors.is_empty() => {
            Line::from(vec![Span::styled(gutter(new, ' '), theme::faint()), Span::styled(text, theme::muted())])
        }
        PatchLine::Context { new, text: t, .. } => {
            let mut spans = vec![Span::styled(gutter(new, ' '), theme::faint())];
            // Nothing is marked on context, so no mark colour.
            spans.extend(text(t, theme::BG));
            Line::from(spans)
        }
        PatchLine::Added { new, text: t } => {
            let mut spans = vec![Span::styled(gutter(new, '+'), Style::new().fg(theme::GREEN))];
            spans.extend(text(t, ADDED_MARK));
            Line::from(spans).style(Style::new().bg(ADDED))
        }
        PatchLine::Removed { old, text: t } => {
            let mut spans = vec![Span::styled(gutter(old, '-'), Style::new().fg(theme::RED))];
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
            later: Later(vec![(1, Lazy::new(Vec::new(), "a.rs", true))]),
        };
        let joined = p(2, 7).then(p(3, 9)).below(vec![Line::raw("top")]);
        assert_eq!((joined.files, joined.hunks), (vec![1, 3], vec![2, 4]));
        assert_eq!(joined.later.0.iter().map(|(row, _)| *row).collect::<Vec<_>>(), [2, 4]);
        assert_eq!(joined.pins, [0, 7, 7, 9, 9, 9]);
        // Without pins of its own, the next one's still line up.
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
        let h = Lazy::new(lines, "notes", true).halves();
        let side = |v: &[Line]| v.iter().map(|l| l.to_string().trim().to_string()).collect::<Vec<_>>();
        assert_eq!(side(&h.old), ["⋯ line 4", "4  a", "5 -b", "6 -c", "7  d", ""]);
        assert_eq!(side(&h.new), ["", "4  a", "5 +B", "", "6  d", "7 +e"]);
        assert_eq!(h.hunks, [0]);
        assert_eq!(h.pins, [u16::MAX, 7, 7, 7, 7, 7]);
    }

    #[test]
    fn colours_as_the_screen_comes_near() {
        let patch = (1..=2000).map(|new| PatchLine::Added { new, text: "let x = 1;".into() }).collect();
        let lazy = Lazy::new(patch, "a.rs", true);
        let mut lines = lazy.lines();
        let mut later = Later::default();
        later.push(0, lazy);
        // `let` is in the text colour until coloured.
        let coloured = |l: &Line| l.spans[1].style.fg != Some(theme::TEXT);
        assert!(!coloured(&lines[0]));
        later.near(&mut lines, 0, 40);
        assert!(coloured(&lines[0]) && coloured(&lines[500]));
        assert!(!coloured(&lines[900]));
        // Scrolled down near the end of what's coloured: more of it.
        later.near(&mut lines, 400, 440);
        assert!(coloured(&lines[900]) && !coloured(&lines[1999]));
        // A jump far past what's coloured stays plain rather than colour all above it.
        later.near(&mut lines, 1960, 2000);
        assert!(!coloured(&lines[1999]));
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
        let drawn = Lazy::new(lines.to_vec(), "notes", true).lines();
        assert_eq!(marked(&drawn[0], REMOVED_MARK), ["2"]);
        assert_eq!(marked(&drawn[1], ADDED_MARK), ["3"]);
        // Nothing it replaces: added whole, nothing marked.
        assert!(marked(&drawn[2], ADDED_MARK).is_empty());
        assert_eq!(drawn[1].to_string(), "    1 +let n = 3;");
        // Without line numbers, only the `+` stays in front.
        assert_eq!(Lazy::new(lines.to_vec(), "notes", false).lines()[1].to_string(), "+let n = 3;");
        assert_eq!(pin(&lines[1], false), 1);
    }
}
