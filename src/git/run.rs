//! Running the git binary for the writes gix doesn't do for us (stash, worktree, branch…).

/// Whether git succeeded, and what it said (stderr first: that's where git talks).
/// Prompts for credentials still reach the terminal: ssh and git ask on /dev/tty.
pub fn git(args: &[String]) -> (bool, String) {
    match crate::process::run("git", args, None) {
        Ok(out) => (out.ok, out.text()),
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

/// `["stash", "drop", "stash@{0}"]` from string slices.
pub fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}
