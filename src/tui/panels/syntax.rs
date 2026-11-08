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

/// Each line's colours, by `path`'s language; none for a language glance doesn't know.
/// The old file's lines (context and removed) and the new file's (context and added)
/// are coloured apart, each from the start of its hunk, so a string or comment that
/// spans lines stays coloured.
pub fn colors(lines: &[PatchLine], path: &str) -> Vec<Colors> {
    let (set, theme) = sets();
    let mut out = vec![Vec::new(); lines.len()];
    let Some(syntax) = syntax(set, path) else { return out };
    let start = || HighlightLines::new(syntax, theme);
    let (mut old, mut new) = (start(), start());
    for (l, out) in lines.iter().zip(&mut out) {
        match l {
            PatchLine::Hunk { .. } => (old, new) = (start(), start()),
            PatchLine::Context { text, .. } => {
                paint(&mut old, set, text);
                *out = paint(&mut new, set, text);
            }
            PatchLine::Removed { text, .. } => *out = paint(&mut old, set, text),
            PatchLine::Added { text, .. } => *out = paint(&mut new, set, text),
        }
    }
    out
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
        let c = colors(&lines, "src/main.rs");
        assert!(c[0].is_empty());
        // `b */` is still the comment opened a line above, `fn` is not.
        let at = |i: usize, byte: usize| c[i].iter().find(|(r, _)| r.contains(&byte)).map(|(_, c)| *c);
        assert_eq!(at(2, 0), at(1, 0));
        assert_ne!(at(2, 5), at(2, 0));
        assert!(colors(&lines, "notes.unknown-ext").iter().all(Vec::is_empty));
    }
}
