//! Reads data from the repository. Never prints anything.

// Remotes, deletes and details are only used by the interactive view.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod branches;
// Patches are drawn only by the interactive log.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod diff;
// Pull requests via gh, shown by the interactive branch view.
#[cfg(feature = "tui")]
pub mod github;
pub mod graph;
pub mod log;
pub mod repos;
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
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

/// Folder name of the working tree, e.g. "aerostack2".
pub fn repo_name(repo: &gix::Repository) -> String {
    // The path can be relative ("."), which has no name until resolved.
    repo.workdir()
        .and_then(|d| d.canonicalize().ok())
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "repo".into())
}

/// Seconds since the Unix epoch, right now.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
