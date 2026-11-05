mod cli;
mod core;
// With the TUI, fzf only provides the clipboard helper; its picker is the no-TUI fallback.
#[cfg_attr(feature = "tui", allow(dead_code))]
mod fzf;
mod patch;
mod rebase;
mod render;
#[cfg(feature = "tui")]
mod tui;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paint = render::Paint::detect(cli.color);

    match cli.command {
        Command::Status => {
            let repo = core::open()?;
            let status = core::status::read(&repo, true)?;
            render::status::print(&status, &paint);
        }
        Command::Log { count, interactive, all } => {
            let repo = core::open()?;
            let count = count.unwrap_or(if interactive { 5000 } else { 20 });
            if interactive {
                return log_interactive(&repo, count, all);
            }
            let commits = core::log::read(&repo, count, all)?;
            render::log::print(&commits, &paint);
        }
        Command::Branches => {
            let repo = core::open()?;
            // Interactive in a terminal; the plain list when piped (or built without the TUI).
            #[cfg(feature = "tui")]
            if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                return tui::branches::run(&repo);
            }
            render::branches::print(&core::branches::read(&repo, false)?, &paint);
        }
        Command::Stash => {
            let repo = core::open()?;
            #[cfg(feature = "tui")]
            if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                return tui::stash::run(&repo);
            }
            render::stash::print(&core::stash::list(&repo)?, &paint);
        }
        Command::Show { rev, patch } => {
            let repo = core::open()?;
            let shown = core::show::read(&repo, &rev)?;
            render::show::print(&shown, &paint);
            if patch {
                println!();
                patch::print(&shown.oid.to_string(), paint.on)?;
            }
        }
        Command::Repos { dir } => {
            render::repos::print(&core::repos::read(&dir)?, &paint);
        }
        Command::Rebase { args } => {
            let code = rebase::run(&args)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Command::RebaseEditor { file, keep_on_quit } => {
            rebase::edit(&file, keep_on_quit)?;
        }
    }
    Ok(())
}

/// Full-screen log; Enter copies the commit id and prints it (so `$(gg log -i)` works).
/// Without the `tui` feature, falls back to fzf.
fn log_interactive(repo: &gix::Repository, count: usize, all: bool) -> Result<()> {
    #[cfg(feature = "tui")]
    let picked = tui::log::run(repo, tui::log::Options { mode: tui::log::Mode::Browse, all, count })?;
    #[cfg(not(feature = "tui"))]
    let picked = {
        anyhow::ensure!(fzf::available(), "`gg log -i` needs fzf installed");
        let commits = core::log::read(repo, count, all)?;
        let lines = render::log::lines(&commits, &render::Paint { on: true, rgb: false });
        fzf::pick_commit(&lines, &core::repo_name(repo), "copy")?
    };
    if let Some(id) = picked {
        fzf::copy(&id);
        println!("{id}");
    }
    Ok(())
}
