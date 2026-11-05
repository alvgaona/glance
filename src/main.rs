mod cli;
mod core;
mod fzf;
mod render;
#[cfg(feature = "tui")]
mod tui;

use anyhow::{Result, bail};
use clap::Parser;

use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paint = render::Paint::detect(cli.color);

    match cli.command {
        Command::Status => {
            let repo = core::open()?;
            let status = core::status::read(&repo)?;
            render::status::print(&status, &paint);
        }
        Command::Log { count, interactive, all } => {
            let repo = core::open()?;
            let count = count.unwrap_or(if interactive { 5000 } else { 20 });
            let commits = core::log::read(&repo, count, all)?;
            if interactive {
                return log_interactive(&repo, &commits);
            }
            render::log::print(&commits, &paint);
        }
        Command::Branches | Command::Show { .. } | Command::Repos => {
            bail!("not implemented yet");
        }
    }
    Ok(())
}

/// fzf picker when installed, otherwise the built-in ratatui list.
fn log_interactive(repo: &gix::Repository, commits: &[core::log::Commit]) -> Result<()> {
    if fzf::available() {
        let lines = render::log::lines(commits, &render::Paint { on: true });
        let label = repo
            .workdir()
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "log".into());
        if let Some(id) = fzf::pick_commit(&lines, &label)? {
            fzf::copy(&id);
            println!("{id}");
        }
        return Ok(());
    }
    #[cfg(feature = "tui")]
    return tui::log::run(commits);
    #[cfg(not(feature = "tui"))]
    bail!("`gg log -i` needs fzf installed");
}
