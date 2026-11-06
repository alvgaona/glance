//! Full-screen views built on ratatui. Only started by interactive commands.
//!
//! `app` and `term` run a screen, `kit` holds generic widgets, `panels` the git-aware
//! pieces several screens share, `screens` one module per command.

mod app;
mod kit;
mod panels;
pub mod screens;
mod state;
mod term;
pub mod theme;
mod watch;

/// First two non-empty lines of git's output, for a toast.
pub(crate) fn first_lines(s: &str) -> String {
    s.lines().filter(|l| !l.trim().is_empty()).take(2).collect::<Vec<_>>().join(" · ")
}
