//! Short text for values, shared by printed output and the full-screen views:
//! "2h ago", "↑2 ↓1", "~/git/app", "M". No colours and no terminal types here.

// Paths and sync arrows only appear in the full-screen views so far.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod paths;
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod sync;
mod time;
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod url;

pub use time::{now, relative_time};

/// Columns `s` takes on a terminal: 2 for `李` and most emoji, 1 for `a`.
pub fn width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// The spaces that fill `s` out to `w` columns (`{:<w$}` counts chars, not columns).
pub fn pad(s: &str, w: usize) -> String {
    " ".repeat(w.saturating_sub(width(s)))
}

use crate::git::status::ChangeKind;

/// The one-letter code `git status -s` uses for a change.
pub fn change_letter(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "A",
        ChangeKind::Modified => "M",
        ChangeKind::Deleted => "D",
        ChangeKind::Renamed => "R",
        ChangeKind::Copied => "C",
        ChangeKind::TypeChange => "T",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pads_by_columns() {
        assert_eq!(super::width("李a"), 3);
        assert_eq!(super::pad("李", 4), "  ");
        assert_eq!(super::pad("long", 2), "");
    }
}
