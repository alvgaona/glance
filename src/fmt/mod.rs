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
