//! `gg st`: always printed.

use anyhow::Result;

use crate::git;
use crate::print::{self, Paint};

pub fn run(paint: &Paint) -> Result<()> {
    let repo = git::open()?;
    print::status::print(&git::status::read(&repo, true)?, paint);
    Ok(())
}
