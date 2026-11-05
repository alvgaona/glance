//! Running the git binary for the writes gix doesn't do for us (stash, worktree, branch…).

/// Whether git succeeded, and what it said (stderr first: that's where git talks).
/// Prompts for credentials still reach the terminal: ssh and git ask on /dev/tty.
pub fn git(args: &[String]) -> (bool, String) {
    git_in(None, args)
}

/// [`git`] in `dir` (or here).
pub fn git_in(dir: Option<&std::path::Path>, args: &[String]) -> (bool, String) {
    match crate::process::run("git", args, dir) {
        Ok(out) => (out.ok(), out.text()),
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

/// [`git_in`] with `input` on git's stdin (a patch for `git apply -`).
pub fn git_with_input(dir: Option<&std::path::Path>, args: &[String], input: &[u8]) -> (bool, String) {
    match crate::process::run_with_input("git", args, dir, input, &[]) {
        Ok(out) => (out.ok(), out.text()),
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

/// [`git`] with `env` set and nothing on stdin, so it can never wait for the terminal:
/// for jobs that run behind a full screen.
pub fn git_no_stdin(args: &[String], env: &[(&str, &str)]) -> (bool, String) {
    match crate::process::run_with_input("git", args, None, b"", env) {
        Ok(out) => (out.ok(), out.text()),
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

/// [`git_in`] on the terminal: for an editor, or a passphrase prompt while signing.
pub fn git_on_terminal(dir: Option<&std::path::Path>, args: &[String]) -> (bool, String) {
    match crate::process::run_on_terminal("git", args, dir) {
        Ok(out) => (out.ok(), out.text()),
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

/// Opens `path` (from the repository root) in git's editor (core.editor, $VISUAL,
/// $EDITOR, vi), on the terminal; started the way git starts it, through the shell.
pub fn edit(repo: &gix::Repository, path: &str) -> (bool, String) {
    let dir = repo.workdir();
    let (ok, editor) = git_in(dir, &args(&["var", "GIT_EDITOR"]));
    if !ok {
        return (false, editor);
    }
    let script = format!("{} \"$@\"", editor.trim());
    match crate::process::run_on_terminal("sh", &args(&["-c", &script, "sh", path]), dir) {
        Ok(out) => (out.ok(), out.text()),
        Err(e) => (false, format!("could not run the editor: {e}")),
    }
}

/// For tests: runs `git args…` in `dir` as author "t <t@t>" and panics if it fails.
#[cfg(test)]
pub fn test_git(dir: &std::path::Path, args: &[&str]) {
    let env = [
        ("GIT_AUTHOR_NAME", "t"),
        ("GIT_AUTHOR_EMAIL", "t@t"),
        ("GIT_COMMITTER_NAME", "t"),
        ("GIT_COMMITTER_EMAIL", "t@t"),
    ];
    let out = crate::process::run_with_input("git", &self::args(args), Some(dir), b"", &env).unwrap();
    assert!(out.ok(), "git {args:?}: {}", out.text());
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
