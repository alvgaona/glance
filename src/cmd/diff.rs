//! `glance diff`: files and diffs on a full screen; piped, plain `git diff`.

use anyhow::Result;

/// Returns the exit code to leave with (git's, when it ran).
pub fn run(args: &[String]) -> Result<i32> {
    #[cfg(feature = "tui")]
    if super::screen_for_stdout() {
        crate::tui::screens::diff::run(args)?;
        return Ok(0);
    }
    let a: Vec<String> = ["diff".to_string()].into_iter().chain(args.iter().cloned()).collect();
    Ok(crate::process::run_attached("git", &a, &[])?.unwrap_or(1))
}
