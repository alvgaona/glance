//! Printed output: what commands write when they don't open a full screen.

pub mod branches;
pub mod log;
mod paint;
pub mod repos;
pub mod stash;
pub mod status;
pub mod worktree;

pub use paint::{Paint, Tone};
