use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "gg", version, about = "A nicer view of git status, log and friends")]
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

    /// Compact commit history.
    #[command(visible_alias = "l")]
    Log {
        /// Number of commits to show [default: 20, or 5000 with -i].
        #[arg(short = 'n', long)]
        count: Option<usize>,

        /// Pick a commit in fzf with a diff preview; Enter copies its id.
        #[arg(short, long)]
        interactive: bool,

        /// Include commits from all branches, remotes and tags, not just HEAD.
        #[arg(short, long)]
        all: bool,
    },

    /// Local branches sorted by last commit.
    #[command(visible_alias = "br")]
    Branches,

    /// One commit with a per-file summary.
    Show {
        /// Revision to show.
        #[arg(default_value = "HEAD")]
        rev: String,

        /// Also print the full diff (through delta when installed).
        #[arg(short, long)]
        patch: bool,
    },

    /// Every repo in a folder, at a glance.
    Repos {
        /// Folder holding the repos.
        #[arg(default_value = ".")]
        dir: std::path::PathBuf,
    },

    /// Interactive rebase with a visual todo editor.
    ///
    ///   gg rebase          this branch's commits (since it split off main),
    ///                      or on main, the commits you haven't pushed yet
    ///   gg rebase 5        the last 5 commits
    ///   gg rebase -p       pick the oldest commit to edit, in fzf
    ///   gg rebase <ref>    anything else goes to `git rebase -i` as is
    ///   gg rebase --continue | --skip | --abort | --edit-todo
    #[command(visible_alias = "rb", verbatim_doc_comment)]
    Rebase {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Used by git as GIT_SEQUENCE_EDITOR during `gg rebase`.
    #[command(name = "__rebase-editor", hide = true)]
    RebaseEditor {
        file: std::path::PathBuf,
        /// Quitting keeps the list unchanged instead of aborting (for --edit-todo).
        #[arg(long)]
        keep_on_quit: bool,
    },
}
