use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "glance", version, about = "A nicer view of git status, log and friends")]
pub struct Cli {
    /// When to use colour.
    #[arg(long, value_enum, default_value_t = ColorMode::Auto, global = true)]
    pub color: ColorMode,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand)]
pub enum Command {
    /// Branch state and changed files, grouped.
    #[command(visible_alias = "st")]
    Status,

    /// Stage files on a full screen, with each file's diff.
    ///
    ///   glance add              full screen: space stages or unstages a file
    ///   glance add <path>…      plain `git add`, then the short status
    #[command(verbatim_doc_comment)]
    Add { paths: Vec<String> },

    /// Commit history: full screen in a terminal, a compact list when piped.
    #[command(visible_alias = "l")]
    Log {
        /// Number of commits to load [default: 5000, or 20 when piped].
        #[arg(short = 'n', long)]
        count: Option<usize>,

        /// Full screen even when piped: `$(glance log -i)` gives the picked commit id.
        /// Without the TUI build, picks in fzf.
        #[arg(short, long)]
        interactive: bool,

        /// Include commits from all branches, remotes and tags, not just HEAD.
        #[arg(short, long)]
        all: bool,
    },

    /// Local branches sorted by last commit.
    #[command(visible_alias = "br")]
    Branches,

    /// Stashes: browse, apply, pop, drop, or turn one into a branch.
    #[command(visible_alias = "sh")]
    Stash,

    /// Worktrees: see them all, add, remove, prune, lock.
    ///
    ///   glance wt               full screen; enter prints the picked folder,
    ///                           so `cd "$(glance wt)"` jumps there
    ///   glance wt -l            one line per worktree
    #[command(visible_alias = "wt", verbatim_doc_comment)]
    Worktrees {
        /// Print the list instead of opening the full screen.
        #[arg(short, long)]
        list: bool,
    },

    /// Remotes: see where each points, add, rename, remove, set URLs, fetch.
    #[command(alias = "remotes")]
    Remote {
        /// Print the list instead of opening the full screen.
        #[arg(short, long)]
        list: bool,
    },

    /// Every repo in a folder, at a glance.
    Repos {
        /// Folder holding the repos.
        #[arg(default_value = ".")]
        dir: std::path::PathBuf,
    },

    /// Interactive rebase with a visual todo editor.
    ///
    ///   glance rebase          this branch's commits (since it split off main),
    ///                          or on main, the commits you haven't pushed yet
    ///   glance rebase 5        the last 5 commits
    ///   glance rebase -p       pick the oldest commit to edit, in fzf
    ///   glance rebase <ref>    anything else goes to `git rebase -i` as is
    ///   glance rebase --continue | --skip | --abort | --edit-todo
    #[command(visible_alias = "rb", verbatim_doc_comment)]
    Rebase {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Used by git as GIT_SEQUENCE_EDITOR during `glance rebase`.
    #[command(name = "__rebase-editor", hide = true)]
    RebaseEditor {
        file: std::path::PathBuf,
        /// Quitting keeps the list unchanged instead of aborting (for --edit-todo).
        #[arg(long)]
        keep_on_quit: bool,
    },
}
