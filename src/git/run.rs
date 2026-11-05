//! Running the git binary for the writes gix doesn't do for us (stash, worktree, branch…).

/// What a git write gave: its output when it worked, else what went wrong
/// (stderr first: that's where git talks).
pub type Ran = Result<String, String>;

fn ran(out: std::io::Result<crate::process::Output>, what: &str) -> Ran {
    match out {
        Ok(o) if o.ok() => Ok(o.text()),
        Ok(o) => Err(o.text()),
        Err(e) => Err(format!("could not run {what}: {e}")),
    }
}

/// Runs git here. Prompts for credentials still reach the terminal: ssh and git ask on /dev/tty.
pub fn git(args: &[String]) -> Ran {
    git_in(None, args)
}

/// [`git`] in `dir` (or here).
pub fn git_in(dir: Option<&std::path::Path>, args: &[String]) -> Ran {
    ran(crate::process::run("git", args, dir), "git")
}

/// [`git_in`] with `input` on git's stdin (a patch for `git apply -`).
pub fn git_with_input(dir: Option<&std::path::Path>, args: &[String], input: &[u8]) -> Ran {
    ran(crate::process::run_with_input("git", args, dir, input, &[]), "git")
}

/// [`git`] with `env` set and nothing on stdin, so it can never wait for the terminal:
/// for jobs that run behind a full screen.
pub fn git_no_stdin(args: &[String], env: &[(&str, &str)]) -> Ran {
    ran(crate::process::run_with_input("git", args, None, b"", env), "git")
}

/// [`git_in`] on the terminal: for an editor, or a passphrase prompt while signing.
pub fn git_on_terminal(dir: Option<&std::path::Path>, args: &[String]) -> Ran {
    ran(crate::process::run_on_terminal("git", args, dir), "git")
}

/// Opens `path` (from the repository root) in git's editor (core.editor, $VISUAL,
/// $EDITOR, vi), on the terminal; started the way git starts it, through the shell.
pub fn edit(repo: &gix::Repository, path: &str) -> Ran {
    let dir = repo.workdir();
    let editor = git_in(dir, &args(&["var", "GIT_EDITOR"]))?;
    let script = format!("{} \"$@\"", editor.trim());
    // `./`: a file called `-x` is a file, not an option.
    let path = format!("./{path}");
    ran(crate::process::run_on_terminal("sh", &args(&["-c", &script, "sh", &path]), dir), "the editor")
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

/// A scratch repo on `main`, removed when dropped. `name` must be unique among tests.
#[cfg(test)]
pub struct Scratch(pub std::path::PathBuf);

#[cfg(test)]
impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("gg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let s = Scratch(dir);
        s.git(&["init", "-q", "-b", "main"]);
        s
    }
    pub fn git(&self, a: &[&str]) {
        test_git(&self.0, a);
    }
    pub fn write(&self, path: &str, text: &str) {
        std::fs::write(self.0.join(path), text).unwrap();
    }
    pub fn repo(&self) -> gix::Repository {
        gix::open(&self.0).unwrap()
    }
    /// Writes `path` and commits it.
    pub fn commit(&self, path: &str, text: &str) {
        self.write(path, text);
        self.git(&["add", path]);
        self.git(&["commit", "-q", "-m", path]);
    }
}

#[cfg(test)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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
