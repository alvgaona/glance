//! `gg rebase`: git does the rebase; gg only replaces the todo-list editor.

use std::path::Path;

use anyhow::{Context, Result};

use crate::git::{self, rebase, todo};

/// Without the full screen there's no todo editor, and git would only fail once it asks.
#[cfg(not(feature = "tui"))]
const NEEDS_TUI: &str = "glance rebase needs the `tui` build: its todo editor is a full screen";

/// Flags that control a rebase already in progress; these never get `-i` added.
const CONTROL: [&str; 6] = ["--continue", "--abort", "--skip", "--quit", "--edit-todo", "--show-current-patch"];

/// Runs `git rebase -i <args>` (or `git rebase --continue` etc.) with gg as the todo editor.
/// Returns git's exit code.
///
/// Shortcuts for choosing which commits to edit:
/// - no arguments: this branch's own commits (since it split off main);
/// - a number `N`: the last N commits;
/// - `-p` / `--pick`: choose the oldest commit to edit.
pub fn run(args: &[String]) -> Result<i32> {
    let control = args.iter().any(|a| CONTROL.contains(&a.as_str()));
    // --continue, --abort and the like don't open the editor, so they still work.
    #[cfg(not(feature = "tui"))]
    if !control || args.iter().any(|a| a == "--edit-todo") {
        anyhow::bail!(NEEDS_TUI);
    }
    let (args, desc) = if control {
        (args.to_vec(), None)
    } else {
        match resolve(args)? {
            Some((args, desc)) => (args, desc),
            None => return Ok(0), // picker closed
        }
    };
    let interactive = args.iter().any(|a| a == "-i" || a == "--interactive");

    let exe = std::env::current_exe().context("can't find the glance binary")?;
    // Git runs this through the shell, so quote the path.
    let quoted = format!("'{}'", exe.display().to_string().replace('\'', r"'\''"));
    // When editing the list mid-rebase, quitting should keep it, not empty it.
    let editor = if args.iter().any(|a| a == "--edit-todo") {
        format!("{quoted} __rebase-editor --keep-on-quit")
    } else {
        format!("{quoted} __rebase-editor")
    };

    let marker = cancel_marker()?;
    let _ = std::fs::remove_file(&marker);
    let code = rebase::start(&args, !control && !interactive, &editor, desc.as_deref()).context("could not run git")?;
    if std::fs::remove_file(&marker).is_ok() {
        // The user quit the editor; git's "nothing to do" error is expected.
        eprintln!("Rebase cancelled. Nothing changed.");
        return Ok(0);
    }
    Ok(code.unwrap_or(1))
}

/// Turns gg's shortcuts into `git rebase` arguments plus a description for the header.
/// `None` means the user closed the picker.
fn resolve(args: &[String]) -> Result<Option<(Vec<String>, Option<String>)>> {
    let repo = git::open()?;
    let (first, rest) = match args.split_first() {
        Some((f, r)) => (Some(f.as_str()), r),
        None => (None, &[][..]),
    };
    let with_rest = |base: String| [vec![base], rest.to_vec()].concat();

    match first {
        None => match rebase::fork_point(&repo)?.or(rebase::unpushed(&repo)?) {
            Some((base, desc)) => Ok(Some((vec![base.to_string()], Some(desc)))),
            None => anyhow::bail!(
                "nothing to pick automatically: this branch has no commits of its own and nothing unpushed.\n\
                 Try `glance rebase 5` (last 5 commits) or `glance rebase -p` (choose where to start)."
            ),
        },
        #[cfg(feature = "tui")]
        Some("-p" | "--pick") => pick(&repo, rest),
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            let n: usize = n.parse().context("count is too large")?;
            let (base, desc) = rebase::last(&repo, n)?;
            Ok(Some((with_rest(base), Some(desc))))
        }
        Some(_) => Ok(Some((args.to_vec(), None))),
    }
}

/// Choose the oldest commit to edit from this branch's history, on the full-screen log.
#[cfg(feature = "tui")]
fn pick(repo: &gix::Repository, rest: &[String]) -> Result<Option<(Vec<String>, Option<String>)>> {
    use crate::tui::screens::log::{Mode, Options, run};
    let picked = run(repo, Options { mode: Mode::Pick, all: false, count: 5000 })?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let (base, desc) = rebase::from_commit(repo, &picked)?;
    Ok(Some(([vec![base], rest.to_vec()].concat(), Some(desc))))
}

/// Left by the editor when the user quits, so `run` can tell a cancel from a failure.
fn cancel_marker() -> Result<std::path::PathBuf> {
    Ok(git::open()?.path().join("glance-rebase-cancelled"))
}

/// Called by git with the todo file path. Writing it back applies the changes.
pub fn edit(file: &Path, keep_on_quit: bool) -> Result<()> {
    let repo = git::open()?;
    let lines = todo::read_file(file)?;
    let (branch, detail) = header(&repo, &lines);

    #[cfg(feature = "tui")]
    {
        use crate::tui::screens::rebase::{Header, Outcome, run};
        match run(&repo, lines, Header { branch, detail }, rebase::newest_first(&repo))? {
            Outcome::Apply(new) => std::fs::write(file, todo::render(&new))?,
            Outcome::Quit if keep_on_quit => {} // leave the file as git wrote it
            // An empty todo list makes git abort the rebase and leave the branch untouched.
            Outcome::Quit => {
                std::fs::write(file, "")?;
                std::fs::write(cancel_marker()?, "")?;
            }
        }
        Ok(())
    }
    #[cfg(not(feature = "tui"))]
    {
        let _ = (lines, branch, detail, keep_on_quit);
        anyhow::bail!(NEEDS_TUI)
    }
}

/// Branch name and a short description of which commits are listed:
/// "4 commits since branching off main", or "onto 1a2b3c4 · 4 commits".
fn header(repo: &gix::Repository, lines: &[todo::Line]) -> (String, String) {
    let p = rebase::progress(repo);
    let n = lines.iter().filter(|l| matches!(l, todo::Line::Commit { .. })).count();
    let detail = match std::env::var(rebase::DESC_ENV) {
        Ok(desc) if !desc.is_empty() => desc,
        _ => {
            let s = if n == 1 { "" } else { "s" };
            let mut d = format!("{n} commit{s}");
            if !p.onto.is_empty() {
                d = format!("onto {} · {d}", p.onto);
            }
            if p.done > 0 {
                d += &format!(" left · {} done", p.done);
            }
            d
        }
    };
    (p.branch, detail)
}
