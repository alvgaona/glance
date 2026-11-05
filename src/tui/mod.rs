//! Full-screen views built on ratatui. Only started by interactive commands.

mod app;
pub mod branches;
mod diffview;
pub mod log;
pub mod rebase;
pub mod stash;
mod stash_new;
mod term;
pub mod theme;
pub mod widgets;
pub mod worktree;

/// First two non-empty lines of git's output, for a toast.
pub(crate) fn first_lines(s: &str) -> String {
    s.lines().filter(|l| !l.trim().is_empty()).take(2).collect::<Vec<_>>().join(" · ")
}
