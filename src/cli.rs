use clap::builder::styling::{Color, Effects, RgbColor, Style, Styles};
use clap::{Parser, Subcommand, ValueEnum};

// Four colours from the full screens' palette (`tui::theme`).
const RED: Color = Color::Rgb(RgbColor(224, 108, 117));
const WHITE: Color = Color::Rgb(RgbColor(238, 238, 238));
const GRAY: Color = Color::Rgb(RgbColor(128, 128, 128));
const PURPLE: Color = Color::Rgb(RgbColor(157, 124, 216));

const HEADER: Style = Style::new().fg_color(Some(RED)).effects(Effects::BOLD);
const LITERAL: Style = Style::new().fg_color(Some(WHITE)).effects(Effects::BOLD);
const COMMENT: Style = Style::new().fg_color(Some(GRAY));

fn styles() -> Styles {
    Styles::styled()
        .header(HEADER)
        .usage(HEADER)
        .literal(LITERAL)
        .placeholder(Style::new().fg_color(Some(PURPLE)))
        .valid(Style::new().fg_color(Some(PURPLE)))
        .invalid(Style::new().fg_color(Some(PURPLE)).effects(Effects::BOLD))
        .error(HEADER)
        .context(COMMENT)
}

/// An "Examples:" section in the help's colours: each command, then what it does.
/// clap takes the colours out when the help isn't going to a terminal.
fn examples(rows: &[(&str, &str)]) -> String {
    let wide = rows.iter().map(|(c, _)| c.chars().count()).max().unwrap_or(0);
    let mut out = format!("{HEADER}Examples:{HEADER:#}\n");
    for (command, what) in rows {
        let pad = " ".repeat(wide - command.chars().count());
        out.push_str(&format!("  {LITERAL}{command}{LITERAL:#}{pad}   {COMMENT}{what}{COMMENT:#}\n"));
    }
    out
}

/// glance's own words after the examples, as a muted note.
fn note(text: &str) -> String {
    format!("\n{COMMENT}{text}{COMMENT:#}")
}

fn top_help() -> String {
    examples(&[
        ("glance add", "stage files, hunks or single lines on a full screen"),
        ("glance d --staged", "what the next commit will hold"),
        ("glance l -a", "history of every branch"),
        ("glance blame src/main.rs", "who last changed each line"),
        ("cd \"$(glance wt)\"", "jump to a worktree"),
        ("glance rb 3", "reword, squash or reorder the last 3 commits"),
    ]) + &note(
        "In a terminal most commands open a full screen, and q quits it.\n\
         Piped, they print plain text, or run the git command they stand for.\n\
         `glance <command> --help` has the details and examples for each.",
    )
}

/// `--color` from the raw arguments: clap draws the help before it parses them.
pub fn help_color(args: impl IntoIterator<Item = String>) -> clap::ColorChoice {
    let args: Vec<String> = args.into_iter().collect();
    let value = args.iter().enumerate().find_map(|(i, a)| match a.strip_prefix("--color") {
        Some("") => args.get(i + 1).map(String::as_str),
        Some(v) => v.strip_prefix('='),
        None => None,
    });
    match value {
        Some("never") => clap::ColorChoice::Never,
        Some("always") => clap::ColorChoice::Always,
        _ => clap::ColorChoice::Auto,
    }
}

#[derive(Parser)]
#[command(
    name = "glance",
    version,
    about = "A nicer view of git: status, staging, diffs, history, branches and more",
    long_about = "A nicer view of git: status, staging, diffs, history, branches and more.\n\n\
        Commands open full screens in a terminal and print plain text when piped,\n\
        so they work in scripts too. Diffs are drawn by delta when it's installed.",
    styles = styles(),
    after_help = top_help()
)]
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
    /// Branch, upstream and changed files, grouped.
    ///
    /// Like `git status`, printed and easier to scan: the branch and how far it is from its
    /// upstream, any merge or rebase in progress, then staged, changed, untracked and
    /// conflicted files with their line counts.
    #[command(
        visible_alias = "st",
        after_help = examples(&[
            ("glance st", "the status"),
            ("glance st --color never", "without colour, for a log file"),
        ])
    )]
    Status,

    /// Stage on a full screen, with each file next to its diff.
    ///
    /// Stage or unstage whole files, single hunks or single lines, discard changes, commit
    /// and amend, and mark conflicts resolved. With paths, it's plain `git add` and then
    /// prints the short status.
    #[command(after_help = examples(&[
        ("glance add", "full screen: space stages or unstages the file"),
        ("glance add src/ README.md", "stage these, then show the status"),
    ]) + &note("On the screen: enter works hunk by hunk, v picks lines, c commits, C amends."))]
    Add {
        /// Files or folders to stage without opening the screen.
        paths: Vec<String>,
    },

    /// Changed files and their diffs, drawn by delta when it's installed.
    ///
    /// The files on the left, the picked one's diff on the right. Switch between unstaged,
    /// staged and everything since HEAD, and change context, whitespace and layout on the
    /// screen. Piped, it is plain `git diff`.
    #[command(
        visible_alias = "d",
        after_help = examples(&[
            ("glance diff", "unstaged changes"),
            ("glance diff --staged", "what's staged"),
            ("glance diff main..", "everything this branch adds"),
            ("glance diff -- src/", "only under src/"),
            ("glance diff | less", "plain `git diff`"),
        ]) + &note("On the screen: 1 2 3 switch what's compared, n N go between hunks, s side by side.")
    )]
    Diff {
        /// Any `git diff` arguments: revisions, `--staged`, `-- <paths>`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, value_name = "GIT DIFF ARGS")]
        args: Vec<String>,
    },

    /// One commit's files and diffs, on the `glance diff` screen.
    #[command(after_help = examples(&[
        ("glance show", "the last commit"),
        ("glance show HEAD~2", "any commit, tag or branch"),
        ("glance show v1.0 | less", "plain `git show`"),
    ]))]
    Show {
        /// The commit to show [default: HEAD].
        rev: Option<String>,
    },

    /// Each line of a file with the commit that last changed it.
    ///
    /// Lines from the same commit are grouped under its id, author and age. Enter opens the
    /// line's commit on the `glance show` screen, at that file. Piped, it is plain
    /// `git blame`.
    #[command(after_help = examples(&[
        ("glance blame src/main.rs", "the file as it is now"),
        ("glance blame HEAD~5 src/main.rs", "the file as it was at that commit"),
    ]) + &note("On the screen: n N go from commit to commit, enter shows it, y copies its id."))]
    Blame {
        /// A file; or a commit, then the file.
        #[arg(required = true, num_args = 1..=2, value_name = "[REV] PATH")]
        args: Vec<String>,
    },

    /// Commit history, with each commit's message, files and diff.
    ///
    /// Full screen in a terminal, a compact list when piped. Enter copies the picked commit's
    /// id and quits, so `$(glance log -i)` gives it to the shell.
    #[command(
        visible_alias = "l",
        after_help = examples(&[
            ("glance log", "this branch's history"),
            ("glance log -a", "every branch, remote and tag"),
            ("glance log -n 10 | cat", "the last 10 commits as text"),
            ("git show $(glance log -i)", "pick a commit for another command"),
        ])
    )]
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

    /// Branches by last commit, with ahead/behind and pull requests.
    ///
    /// Check out, rename, delete locally or on the remote, clean up merged or gone branches,
    /// fetch, and open a branch's pull request (with `gh` installed).
    #[command(
        visible_alias = "br",
        after_help = examples(&[
            ("glance br", "full screen"),
            ("glance br | cat", "one line per branch"),
        ]) + &note("On the screen: m selects every merged or gone branch, then d deletes them.")
    )]
    Branches,

    /// Stashes: browse, apply, pop, drop, or turn one into a branch.
    ///
    /// Also stashes just the files you pick, only what's staged, or only what isn't.
    #[command(
        visible_alias = "sh",
        after_help = examples(&[
            ("glance sh", "full screen"),
            ("glance sh | cat", "one line per stash"),
        ]) + &note("On the screen: n stashes picked files, s only staged, u only unstaged.")
    )]
    Stash,

    /// Worktrees: see them all, add, remove, prune, lock.
    #[command(
        visible_alias = "wt",
        after_help = examples(&[
            ("glance wt", "full screen; enter prints the picked folder"),
            ("cd \"$(glance wt)\"", "jump to a worktree"),
            ("glance wt -l", "one line per worktree"),
        ])
    )]
    Worktrees {
        /// Print the list instead of opening the full screen.
        #[arg(short, long)]
        list: bool,
    },

    /// Remotes: see where each points, add, rename, remove, set URLs, fetch.
    #[command(
        alias = "remotes",
        after_help = examples(&[
            ("glance remote", "full screen"),
            ("glance remote -l", "one line per remote"),
        ])
    )]
    Remote {
        /// Print the list instead of opening the full screen.
        #[arg(short, long)]
        list: bool,
    },

    /// Every repository in a folder, one line each.
    ///
    /// Branch, last commit, merges or rebases in progress, changes, upstream and stashes.
    #[command(after_help = examples(&[
        ("glance repos", "the repositories in this folder"),
        ("glance repos ~/code", "the ones in ~/code"),
    ]))]
    Repos {
        /// Folder holding the repos.
        #[arg(default_value = ".")]
        dir: std::path::PathBuf,
    },

    /// Interactive rebase with a visual todo editor instead of a text file.
    ///
    /// Without arguments it takes this branch's commits since it split off main, or on main
    /// the ones not pushed yet. Pick, reword, edit, squash, fixup, drop and reorder them,
    /// then ctrl+s runs the rebase.
    #[command(
        visible_alias = "rb",
        after_help = examples(&[
            ("glance rebase", "this branch's commits"),
            ("glance rebase 5", "the last 5 commits"),
            ("glance rebase -p", "pick the oldest commit to edit from the log"),
            ("glance rebase main", "anything else goes to `git rebase -i` as is"),
            ("glance rebase --continue", "also --skip, --abort, --edit-todo"),
        ]) + &note("The order follows `git config glance.rebaseOrder` (newest or oldest first).")
    )]
    Rebase {
        /// A count, `-p`, or any `git rebase` arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, value_name = "ARGS")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn color_never_reaches_the_help() {
        assert_eq!(help_color(["glance", "--color", "never", "-h"].map(String::from)), clap::ColorChoice::Never);
        assert_eq!(help_color(["glance", "log", "--color=always"].map(String::from)), clap::ColorChoice::Always);
        assert_eq!(help_color(["glance", "-h"].map(String::from)), clap::ColorChoice::Auto);
    }

    #[test]
    fn the_cli_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn examples_line_up() {
        let text = examples(&[("a", "one"), ("abc", "two")]);
        let plain: String = anstream_strip(&text);
        assert_eq!(plain, "Examples:\n  a     one\n  abc   two\n");
    }

    /// The text without colour codes.
    fn anstream_strip(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                chars.by_ref().find(|&c| c == 'm');
            } else {
                out.push(c);
            }
        }
        out
    }
}
