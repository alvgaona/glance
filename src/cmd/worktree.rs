//! `gg wt`: full screen whenever someone is at the keyboard (stdout is usually captured:
//! `cd "$(gg wt)"`); enter prints the picked folder. `-l` prints the list instead.

use anyhow::Result;

use crate::git;
use crate::print::{self, Paint};

pub fn run(list: bool, paint: &Paint) -> Result<()> {
    let repo = git::open()?;
    #[cfg(feature = "tui")]
    if !list && super::screen_for_keyboard() {
        if let Some(path) = crate::tui::worktree::run(&repo)? {
            println!("{}", path.display());
        }
        return Ok(());
    }
    let _ = list;
    print::worktree::print(&git::worktree::list(&repo)?, paint);
    Ok(())
}
