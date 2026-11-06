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

pub fn blame(rev: Option<&str>, path: &str) -> Result<i32> {
    #[cfg(feature = "tui")]
    if super::screen_for_stdout() {
        let repo = crate::git::open()?;
        // Said here, not on an empty screen.
        match rev {
            Some(rev) if repo.rev_parse_single(rev).is_err() => anyhow::bail!("unknown revision `{rev}`"),
            None if !std::path::Path::new(path).is_file() => {
                if repo.rev_parse_single(path).is_ok() {
                    anyhow::bail!("`{path}` is a commit; blame takes a file: glance blame [<rev>] <path>");
                }
                anyhow::bail!("no such file: {path}");
            }
            _ => {}
        }
        crate::tui::screens::blame::run(repo, rev, path)?;
        return Ok(0);
    }
    let args: Vec<String> = rev.into_iter().chain(["--", path]).map(String::from).collect();
    git("blame", &args)
}

fn git(command: &str, args: &[String]) -> Result<i32> {
    let a: Vec<String> = std::iter::once(command.to_string()).chain(args.iter().cloned()).collect();
    Ok(crate::process::run_attached("git", &a, &[])?.unwrap_or(1))
}
