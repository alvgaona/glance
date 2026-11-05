//! `gg stash`: full screen in a terminal, one line per stash when piped.

use anyhow::Result;

use crate::git;
use crate::print::{self, Paint};

pub fn run(paint: &Paint) -> Result<()> {
    let repo = git::open()?;
    #[cfg(feature = "tui")]
    if super::screen_for_stdout() {
        return crate::tui::screens::stash::run(&repo);
    }
    print::stash::print(&git::stash::list(&repo)?, paint);
    Ok(())
}
