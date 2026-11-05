//! Running the git binary for the writes gix doesn't do for us (stash, worktree, branch…).

/// Whether git succeeded, and what it said (stderr first: that's where git talks).
/// Prompts for credentials still reach the terminal: ssh and git ask on /dev/tty.
pub fn git(args: &[String]) -> (bool, String) {
    git_in(None, args)
}

/// [`git`] in `dir` (or here).
pub fn git_in(dir: Option<&std::path::Path>, args: &[String]) -> (bool, String) {
    match crate::process::run("git", args, dir) {
        Ok(out) => (out.ok, out.text()),
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

/// [`git_in`] with `input` on git's stdin (a patch for `git apply -`).
pub fn git_with_input(dir: Option<&std::path::Path>, args: &[String], input: &[u8]) -> (bool, String) {
    match crate::process::run_with_input("git", args, dir, input) {
        Ok(out) => (out.ok, out.text()),
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

/// [`git_in`] on the terminal: for an editor, or a passphrase prompt while signing.
pub fn git_on_terminal(dir: Option<&std::path::Path>, args: &[String]) -> (bool, String) {
    match crate::process::run_on_terminal("git", args, dir) {
        Ok(out) => (out.ok, out.text()),
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

/// A path from the repository root, as a pathspec that means exactly that file even
/// when gg runs in a subfolder or the name has `*` in it.
pub fn top(path: &str) -> String {
    format!(":(top,literal){path}")
}

/// `["stash", "drop", "stash@{0}"]` from string slices.
pub fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}
