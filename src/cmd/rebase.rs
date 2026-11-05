//! `gg rebase`: git does the rebase; gg only replaces the todo-list editor.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::git::{self, rebase, todo};

/// Flags that control a rebase already in progress; these never get `-i` added.
const CONTROL: [&str; 6] = ["--continue", "--abort", "--skip", "--quit", "--edit-todo", "--show-current-patch"];

/// Tells the editor how the commits were chosen, for its header.
const DESC_ENV: &str = "GG_REBASE_DESC";

/// Runs `git rebase -i <args>` (or `git rebase --continue` etc.) with gg as the todo editor.
/// Returns git's exit code.
///
/// Shortcuts for choosing which commits to edit:
/// - no arguments: this branch's own commits (since it split off main);
/// - a number `N`: the last N commits;
/// - `-p` / `--pick`: choose the oldest commit to edit.
pub fn run(args: &[String]) -> Result<i32> {
    let control = args.iter().any(|a| CONTROL.contains(&a.as_str()));
    let (args, desc) = if control {
        (args.to_vec(), None)
    } else {
        match resolve(args)? {
            Some((args, desc)) => (args, desc),
            None => return Ok(0), // picker closed
        }
    };
    let interactive = args.iter().any(|a| a == "-i" || a == "--interactive");

    let exe = std::env::current_exe().context("can't find the gg binary")?;
    // Git runs this through the shell, so quote the path.
    let quoted = format!("'{}'", exe.display().to_string().replace('\'', r"'\''"));
    // When editing the list mid-rebase, quitting should keep it, not empty it.
    let editor = if args.iter().any(|a| a == "--edit-todo") {
        format!("{quoted} __rebase-editor --keep-on-quit")
    } else {
        format!("{quoted} __rebase-editor")
    };

    let mut cmd = Command::new("git");
    cmd.arg("rebase");
    if !control && !interactive {
        cmd.arg("-i");
    }
    cmd.args(&args).env("GIT_SEQUENCE_EDITOR", editor);
    match &desc {
        Some(d) => cmd.env(DESC_ENV, d),
        None => cmd.env_remove(DESC_ENV),
    };

    let marker = cancel_marker()?;
    let _ = std::fs::remove_file(&marker);
    let status = cmd.status().context("could not run git")?;
    if std::fs::remove_file(&marker).is_ok() {
        // The user quit the editor; git's "nothing to do" error is expected.
        eprintln!("Rebase cancelled. Nothing changed.");
        return Ok(0);
    }
    Ok(status.code().unwrap_or(1))
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
                 Try `gg rebase 5` (last 5 commits) or `gg rebase -p` (choose where to start)."
            ),
        },
        Some("-p" | "--pick") => pick(&repo, rest),
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            let n: usize = n.parse().context("count is too large")?;
            if n == 0 {
                anyhow::bail!("nothing to rebase: 0 commits");
            }
            let head = repo.head_id().context("this branch has no commits yet")?.detach();
            let available = repo.rev_walk([head]).first_parent_only().all()?.take(n + 1).count();
            let desc = Some(format!("last {n} commit{}", if n == 1 { "" } else { "s" }));
            match available.cmp(&n) {
                std::cmp::Ordering::Greater => Ok(Some((with_rest(format!("HEAD~{n}")), desc))),
                // Exactly the whole history: git needs --root for that.
                std::cmp::Ordering::Equal => Ok(Some((with_rest("--root".into()), desc))),
                std::cmp::Ordering::Less => {
                    anyhow::bail!("this branch only has {available} commits")
                }
            }
        }
        Some(_) => Ok(Some((args.to_vec(), None))),
    }
}

/// Choose the oldest commit to edit from this branch's history (full-screen log, or fzf).
fn pick(repo: &gix::Repository, rest: &[String]) -> Result<Option<(Vec<String>, Option<String>)>> {
    #[cfg(feature = "tui")]
    let picked = {
        use crate::tui::screens::log::{Mode, Options, run};
        run(repo, Options { mode: Mode::Pick, all: false, count: 5000 })?
    };
    #[cfg(not(feature = "tui"))]
    let picked = {
        let commits = git::log::read(repo, 5000, false)?;
        let lines = crate::print::log::lines(&commits, &crate::print::Paint { on: true, rgb: false });
        let label = format!("{} · rebase: choose the oldest commit to edit", git::repo_name(repo));
        crate::fzf::pick_commit(&lines, &label, "rebase from here")?
    };
    let Some(picked) = picked else {
        return Ok(None);
    };
    let commit = repo.rev_parse_single(picked.as_str())?.object()?.peel_to_commit()?;
    let subject = commit.message()?.summary().to_string();
    let desc = Some(format!("from {picked} {subject}"));
    let base = match commit.parent_ids().next() {
        Some(parent) => parent.to_string(),
        None => "--root".into(),
    };
    Ok(Some(([vec![base], rest.to_vec()].concat(), desc)))
}

/// Left by the editor when the user quits, so `run` can tell a cancel from a failure.
fn cancel_marker() -> Result<std::path::PathBuf> {
    Ok(git::open()?.path().join("gg-rebase-cancelled"))
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
        anyhow::bail!("the rebase editor needs gg built with the `tui` feature")
    }
}

/// Branch name and a short description of which commits are listed:
/// "4 commits since branching off main", or "onto 1a2b3c4 · 4 commits".
fn header(repo: &gix::Repository, lines: &[todo::Line]) -> (String, String) {
    let p = rebase::progress(repo);
    let n = lines.iter().filter(|l| matches!(l, todo::Line::Commit { .. })).count();
    let detail = match std::env::var(DESC_ENV) {
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
