mod cli;
mod clipboard;
mod cmd;
#[cfg(feature = "tui")]
mod delta;
mod fmt;
// The commit picker when built without the full-screen views.
#[cfg_attr(feature = "tui", allow(dead_code))]
mod fzf;
mod git;
// Pull requests via gh, shown by the branch screen.
#[cfg(feature = "tui")]
mod github;
mod print;
// Every program glance starts goes through here; some helpers are only used by the full screens.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
mod process;
#[cfg(feature = "tui")]
mod tui;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paint = print::Paint::detect(cli.color);
    match cli.command {
        Command::Status => cmd::status::run(&paint),
        Command::Add { paths } => cmd::add::run(&paths, &paint),
        Command::Diff { args } => {
            let code = cmd::diff::run(&args)?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Command::Log { count, interactive, all } => cmd::log::run(count, interactive, all, &paint),
        Command::Branches => cmd::branches::run(&paint),
        Command::Stash => cmd::stash::run(&paint),
        Command::Worktrees { list } => cmd::worktree::run(list, &paint),
        Command::Remote { list } => cmd::remote::run(list, &paint),
        Command::Repos { dir } => cmd::repos::run(&dir, &paint),
        Command::Rebase { args } => {
            let code = cmd::rebase::run(&args)?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Command::RebaseEditor { file, keep_on_quit } => cmd::rebase::edit(&file, keep_on_quit),
    }
}
