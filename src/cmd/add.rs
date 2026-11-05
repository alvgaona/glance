//! `gg add`: the staging screen in a terminal. `gg add <paths>` is plain `git add`
//! followed by the short status, so typing it the old way keeps working.

use anyhow::Result;

use crate::git;
use crate::print::{self, Paint};

pub fn run(paths: &[String], paint: &Paint) -> Result<()> {
    let repo = git::open()?;
    if !paths.is_empty() {
        let out = git::stage::add_typed(paths).map_err(anyhow::Error::msg)?;
        if !out.is_empty() {
            eprintln!("{out}");
        }
        print::status::print(&git::status::read(&repo, true)?, paint);
        return Ok(());
    }
    #[cfg(feature = "tui")]
    if super::screen_for_stdout() {
        return crate::tui::screens::add::run(repo);
    }
    print::status::print(&git::status::read(&repo, true)?, paint);
    eprintln!("glance add opens a full screen in a terminal; or name the files: glance add <path>…");
    Ok(())
}
