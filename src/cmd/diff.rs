//! `glance diff`, `glance show` and `glance blame`: full screens; piped, plain
//! `git diff`, `git show` and `git blame`.

use anyhow::Result;

/// Returns the exit code to leave with (git's, when it ran).
pub fn run(args: &[String]) -> Result<i32> {
    #[cfg(feature = "tui")]
    if super::screen_for_stdout() {
        let target = crate::git::compare::Target::from_args(args);
        crate::tui::screens::diff::run(crate::git::open()?, target, None)?;
        return Ok(0);
    }
    git("diff", args)
}

pub fn show(rev: &str) -> Result<i32> {
    #[cfg(feature = "tui")]
    if super::screen_for_stdout() {
        let repo = crate::git::open()?;
        let target = crate::git::compare::commit(&repo, rev)?;
        crate::tui::screens::diff::run(repo, target, None)?;
        return Ok(0);
    }
    git("show", &[rev.to_string()])
}

pub fn blame(path: &str) -> Result<i32> {
    #[cfg(feature = "tui")]
    if super::screen_for_stdout() {
        crate::tui::screens::blame::run(crate::git::open()?, path)?;
        return Ok(0);
    }
    git("blame", &[path.to_string()])
}

fn git(command: &str, args: &[String]) -> Result<i32> {
    let a: Vec<String> = std::iter::once(command.to_string()).chain(args.iter().cloned()).collect();
    Ok(crate::process::run_attached("git", &a, &[])?.unwrap_or(1))
}
