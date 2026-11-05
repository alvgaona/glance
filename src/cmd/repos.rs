//! `gg repos`: every repo in a folder, printed.

use std::path::Path;

use anyhow::Result;

use crate::git;
use crate::print::{self, Paint};

pub fn run(dir: &Path, paint: &Paint) -> Result<()> {
    print::repos::print(&git::repos::read(dir)?, paint);
    Ok(())
}
