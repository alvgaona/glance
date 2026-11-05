//! `gg log`: full screen in a terminal, the graph and one line per commit when piped.
//! `-i` forces the screen so `$(gg log -i)` can pick a commit id.

use anyhow::Result;

use crate::git;
use crate::print::{self, Paint};

pub fn run(count: Option<usize>, interactive: bool, all: bool, paint: &Paint) -> Result<()> {
    let repo = git::open()?;
    let full_screen = interactive || super::screen_for_stdout();
    let count = count.unwrap_or(if full_screen { 5000 } else { 20 });
    if !full_screen {
        print::log::print(&git::log::read(&repo, count, all)?, paint);
        return Ok(());
    }
    if let Some(id) = pick(&repo, count, all)? {
        crate::clipboard::copy(&id);
        println!("{id}");
    }
    Ok(())
}

/// The picked commit id. Without the full-screen views, fzf does the picking.
fn pick(repo: &gix::Repository, count: usize, all: bool) -> Result<Option<String>> {
    #[cfg(feature = "tui")]
    {
        use crate::tui::screens::log::{Mode, Options};
        crate::tui::screens::log::run(repo, Options { mode: Mode::Browse, all, count })
    }
    #[cfg(not(feature = "tui"))]
    {
        anyhow::ensure!(crate::fzf::available(), "`gg log -i` needs fzf installed");
        let commits = git::log::read(repo, count, all)?;
        let lines = print::log::lines(&commits, &Paint { on: true, rgb: false });
        crate::fzf::pick_commit(&lines, &git::repo_name(repo), "copy")
    }
}
