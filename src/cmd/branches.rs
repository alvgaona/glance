//! `gg br`: full screen in a terminal, one line per branch when piped.

use anyhow::Result;

use crate::git;
use crate::print::{self, Paint};

pub fn run(paint: &Paint) -> Result<()> {
    let repo = git::open()?;
    #[cfg(feature = "tui")]
    if super::screen_for_stdout() {
        return crate::tui::screens::branches::run(&repo);
    }
    print::branches::print(&git::branches::read(&repo, false)?, paint);
    Ok(())
}
