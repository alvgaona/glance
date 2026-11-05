mod cli;
mod core;
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
            let commits = core::log::read(&repo, count, all)?;
            if interactive {
                return log_interactive(&repo, &commits);
            }
            render::log::print(&commits, &paint);
        }
        Command::Branches => {
            let repo = core::open()?;
            render::branches::print(&core::branches::read(&repo)?, &paint);
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
    anyhow::bail!("`gg log -i` needs fzf installed");
}
