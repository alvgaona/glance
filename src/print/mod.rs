//! Printed output: what commands write when they don't open a full screen.

pub mod branches;
pub mod log;
mod paint;
pub mod remote;
pub mod repos;
pub mod stash;
pub mod status;
pub mod worktree;

pub use paint::{Paint, Tone};

/// Writes `lines` to stdout. Stops quietly once the reader is gone (`| head`), where
/// `println!` would panic.
pub fn out<T: std::fmt::Display>(lines: impl IntoIterator<Item = T>) {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    for l in lines {
        if writeln!(stdout, "{l}").is_err() {
            return;
        }
    }
}
