//! `gg rebase`: git does the rebase; gg only replaces the todo-list editor.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::core::{self, todo};

/// Flags that control a rebase already in progress; these never get `-i` added.
const CONTROL: [&str; 6] = ["--continue", "--abort", "--skip", "--quit", "--edit-todo", "--show-current-patch"];

/// Runs `git rebase -i <args>` (or `git rebase --continue` etc.) with gg as the todo editor.
/// Returns git's exit code.
pub fn run(args: &[String]) -> Result<i32> {
    let control = args.iter().any(|a| CONTROL.contains(&a.as_str()));
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
    cmd.args(args).env("GIT_SEQUENCE_EDITOR", editor);

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

/// Left by the editor when the user quits, so `run` can tell a cancel from a failure.
fn cancel_marker() -> Result<std::path::PathBuf> {
    Ok(core::open()?.path().join("gg-rebase-cancelled"))
}

/// Called by git with the todo file path. Writing it back applies the changes.
pub fn edit(file: &Path, keep_on_quit: bool) -> Result<()> {
    let repo = core::open()?;
    let lines = todo::read_file(file)?;
    let header = header(&repo, &lines);

    #[cfg(feature = "tui")]
    {
        use crate::tui::rebase::{Outcome, run};
        match run(&repo, lines, header)? {
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
        let _ = (lines, header, keep_on_quit);
        anyhow::bail!("the rebase editor needs gg built with the `tui` feature")
    }
}

/// "Rebase feat/x onto 1a2b3c4 · 5 commits"
fn header(repo: &gix::Repository, lines: &[todo::Line]) -> String {
    let dir = repo.path().join("rebase-merge");
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string());
    let branch = read("head-name")
        .map(|h| h.trim_start_matches("refs/heads/").to_string())
        .unwrap_or_else(|| "HEAD".into());
    let onto = read("onto").map(|o| o.chars().take(7).collect::<String>()).unwrap_or_default();
    let n = lines.iter().filter(|l| matches!(l, todo::Line::Commit { .. })).count();
    let done = read("done").map(|d| todo::parse(&d).len()).unwrap_or(0);
    let mut s = format!("Rebase {branch}");
    if !onto.is_empty() {
        s += &format!(" onto {onto}");
    }
    s += &format!(" · {n} commit{}", if n == 1 { "" } else { "s" });
    if done > 0 {
        s += &format!(" left · {done} done");
    }
    s
}
