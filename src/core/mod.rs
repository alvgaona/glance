//! Reads data from the repository. Never prints anything.

pub mod log;
pub mod status;

use anyhow::{Context, Result};

pub fn open() -> Result<gix::Repository> {
    gix::discover(".").context("not inside a git repository")
}

/// Seconds since the Unix epoch, right now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
