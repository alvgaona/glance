//! Everything that reads or writes the repository. Never prints or draws.

// Remotes, deletes and details are only used by the interactive view.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod branches;
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod commit;
// Patches are drawn only by the interactive views.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod diff;
pub mod log;
// The editor's settings are only read with the full-screen views.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod rebase;
// Branches, times and the writes are only used by the interactive view.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod remote;
pub mod repos;
// The writes are only made from the full-screen views.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod run;
// Staging is only done from the interactive view.
// TODO(gg add step 3): back to `cfg_attr(not(feature = "tui"), …)` once the screen uses it.
#[allow(dead_code)]
pub mod stage;
// Files and details are only shown by the interactive view.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod stash;
pub mod status;
// Only the TUI rebase editor edits the todo list.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod todo;
// Per-file details and patches are only shown by the interactive views.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod tree;
// Adding worktrees and their state are only used by the interactive view.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub mod worktree;

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
