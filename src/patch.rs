//! `gg show -p`: the full diff. Drawing patches is delta's job, so this hands off
//! to `git show | delta` rather than reimplementing it.

use std::process::{Command, Stdio};

use anyhow::{Context, Result};

pub fn print(rev: &str, color: bool) -> Result<()> {
    let color_arg = if color { "--color=always" } else { "--color=never" };
    let use_delta = color && crate::fzf::which("delta");

    let mut git = Command::new("git")
        .args(["--no-pager", "show", "--format=", "--patch", color_arg, rev])
        .stdout(if use_delta { Stdio::piped() } else { Stdio::inherit() })
        .spawn()
        .context("could not run git")?;

    if use_delta {
        let input = git.stdout.take().expect("stdout is piped");
        Command::new("delta").args(["--paging=never"]).stdin(input).status().context("could not run delta")?;
    }
    git.wait()?;
    Ok(())
}
