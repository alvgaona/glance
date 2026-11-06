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
use clap::{CommandFactory, FromArgMatches};

use cli::{Cli, Command};

fn main() -> Result<()> {
    // The help and errors follow `--color` too.
    let matches = Cli::command().color(cli::help_color(std::env::args())).get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
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
        Command::Show { rev } => {
            let code = cmd::diff::show(rev.as_deref().unwrap_or("HEAD"))?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Command::Blame { args } => {
            let (rev, path) = match args.as_slice() {
                [rev, path] => (Some(rev.as_str()), path.as_str()),
                _ => (None, args[0].as_str()),
            };
            let code = cmd::diff::blame(rev, path)?;
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
