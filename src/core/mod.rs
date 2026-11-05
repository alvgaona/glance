//! Reads data from the repository. Never prints anything.

pub mod branches;
pub mod diff;
pub mod graph;
pub mod log;
pub mod repos;
pub mod show;
pub mod status;
// Only the TUI rebase editor edits the todo list.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod todo;

use anyhow::{Context, Result};

pub fn open() -> Result<gix::Repository> {
    let mut repo = gix::discover(".").context("not inside a git repository")?;
    // The log walk reads each commit twice (walk, then details); a cache avoids the second read.
    repo.object_cache_size_if_unset(16 << 20);
    Ok(repo)
}

/// Seconds since the Unix epoch, right now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
