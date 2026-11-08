//! Code coloured by its language, for diff lines.

use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;

use ratatui::style::Color;
use syntect::easy::HighlightLines;
use syntect::highlighting::{ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::git::diff::PatchLine;
use crate::tui::theme;

/// Pieces of a line's text and their colour.
pub type Colors = Vec<(Range<usize>, Color)>;

/// Longer lines (minified code, say) are left plain: highlighting them is slow.
const MAX_LINE: usize = 1000;

fn sets() -> &'static (SyntaxSet, Theme) {
    static SETS: OnceLock<(SyntaxSet, Theme)> = OnceLock::new();
    SETS.get_or_init(|| (SyntaxSet::load_defaults_newlines(), palette()))
}

/// Code in glance's own colours, everything else in its text colour.
fn palette() -> Theme {
    let rgb = |c: Color| match c {
        Color::Rgb(r, g, b) => syntect::highlighting::Color { r, g, b, a: 0xff },
        _ => unreachable!("glance's theme is all RGB"),
    };
    let item = |scopes: &str, c: Color| ThemeItem {
        scope: scopes.parse::<ScopeSelectors>().expect("valid scope selectors"),
        style: StyleModifier { foreground: Some(rgb(c)), ..Default::default() },
    };
    Theme {
        settings: ThemeSettings { foreground: Some(rgb(theme::TEXT)), ..Default::default() },
        scopes: vec![
            item("keyword, storage, keyword.operator.word", theme::PURPLE),
            item("string, constant.character, markup.raw", theme::GREEN),
            item("constant, support.constant, variable.language", theme::ACCENT),
            item("entity.name.function, support.function, meta.function-call entity.name", theme::BLUE),
            item(
                "entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, support.type, support.class, storage.type.primitive, entity.other.inherited-class",
                theme::YELLOW,
            ),
            item("entity.name.tag, markup.heading, entity.name.section", theme::RED),
            item("entity.other.attribute-name, meta.attribute, entity.name.label", theme::YELLOW),
            item("comment, punctuation.definition.comment", Color::Rgb(150, 150, 150)),
        ],
        ..Default::default()
    }
}

fn syntax<'a>(set: &'a SyntaxSet, path: &str) -> Option<&'a SyntaxReference> {
    let path = Path::new(path);
    let by = |s: Option<&std::ffi::OsStr>| s.and_then(|s| s.to_str()).and_then(|s| set.find_syntax_by_extension(s));
    // The name first, for `Makefile` and the like.
    by(path.file_name()).or_else(|| by(path.extension())).filter(|s| s.name != "Plain Text")
}

/// Colours a diff's lines in order, as far as asked, so a long diff is coloured as it
/// scrolls into view. The old file's lines (context and removed) and the new file's
/// (context and added) are coloured apart, each from the start of its hunk, so a string
/// or comment that spans lines stays coloured.
pub struct Painter {
    /// The language and where it's at in the old file and the new; none for a language
    /// glance doesn't know, which stays uncoloured.
    state: Option<(&'static SyntaxReference, HighlightLines<'static>, HighlightLines<'static>)>,
    colors: Vec<Colors>,
}

impl Painter {
    /// For a file at `path`, by its language.
    pub fn new(path: &str) -> Self {
        let (set, theme) = sets();
        let state = syntax(set, path).map(|s| (s, HighlightLines::new(s, theme), HighlightLines::new(s, theme)));
        Painter { state, colors: Vec::new() }
    }

    /// Line `i`'s colours; none past where colouring stopped.
    pub fn colors(&self, i: usize) -> &[(Range<usize>, Color)] {
        self.colors.get(i).map_or(&[], Vec::as_slice)
    }

    /// Lines coloured so far, from the first.
    pub fn done(&self) -> usize {
        self.colors.len()
    }

    /// Colours `lines` up to line `n`. False when there was nothing more to colour.
    pub fn reach(&mut self, lines: &[PatchLine], n: usize) -> bool {
        let (set, theme) = sets();
        let Some((syntax, old, new)) = &mut self.state else { return false };
        let from = self.colors.len();
        for l in lines.iter().take(n).skip(from) {
            let c = match l {
                PatchLine::Hunk { .. } => {
                    (*old, *new) = (HighlightLines::new(syntax, theme), HighlightLines::new(syntax, theme));
                    Vec::new()
                }
                PatchLine::Context { text, .. } => {
                    paint(old, set, text);
                    paint(new, set, text)
                }
                PatchLine::Removed { text, .. } => paint(old, set, text),
                PatchLine::Added { text, .. } => paint(new, set, text),
            };
            self.colors.push(c);
        }
        self.colors.len() > from
    }
}

fn paint(h: &mut HighlightLines, set: &SyntaxSet, text: &str) -> Colors {
    if text.len() > MAX_LINE {
        return Vec::new();
    }
    let line = format!("{text}\n");
    let Ok(pieces) = h.highlight_line(&line, set) else { return Vec::new() };
    let mut at = 0;
    pieces
        .into_iter()
        .map(|(style, piece)| {
            let range = at..(at + piece.len()).min(text.len());
            at += piece.len();
            let c = style.foreground;
            (range, Color::Rgb(c.r, c.g, c.b))
        })
        .filter(|(r, _)| !r.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_by_language_across_lines() {
        let lines = [
            PatchLine::Hunk { old_start: 1, new_start: 1 },
            PatchLine::Context { old: 1, new: 1, text: "/* a".into() },
            PatchLine::Added { new: 2, text: "b */ fn x() {}".into() },
        ];
        let mut p = Painter::new("src/main.rs");
        // Only as far as asked, then on from there.
        assert!(p.reach(&lines, 2) && p.done() == 2);
        assert!(p.reach(&lines, 99) && p.done() == 3);
        assert!(!p.reach(&lines, 99));
        assert!(p.colors(0).is_empty());
        // `b */` is still the comment opened a line above, `fn` is not.
        let at = |i: usize, byte: usize| p.colors(i).iter().find(|(r, _)| r.contains(&byte)).map(|(_, c)| *c);
        assert_eq!(at(2, 0), at(1, 0));
        assert_ne!(at(2, 5), at(2, 0));
        let mut plain = Painter::new("notes.unknown-ext");
        assert!(!plain.reach(&lines, 99) && plain.colors(2).is_empty());
    }
}
