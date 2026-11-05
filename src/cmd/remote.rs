//! `gg remote`: full screen in a terminal, one line per remote when piped or with `-l`.

use anyhow::Result;

use crate::git;
use crate::print::{self, Paint};

pub fn run(list: bool, paint: &Paint) -> Result<()> {
    let repo = git::open()?;
    #[cfg(feature = "tui")]
    if !list && super::screen_for_stdout() {
        return crate::tui::screens::remote::run(repo);
    }
    let _ = list;
    print::remote::print(&git::remote::list(&repo)?, paint);
    Ok(())
}
