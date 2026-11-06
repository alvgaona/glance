//! Commands glance doesn't run reach git as typed, with git's output and exit code.

// A test crate can't reach src/process.rs; it runs the built binary and git directly.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// An empty repository in a temporary folder, removed afterwards.
struct Repo(PathBuf);

impl Repo {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("glance-pass-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = Repo(dir);
        for args in [&["init", "-q", "-b", "main"][..], &["config", "user.name", "t"], &["config", "user.email", "t@t"]]
        {
            repo.git(args);
        }
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }
    fn git(&self, args: &[&str]) -> Output {
        Command::new("git").args(args).current_dir(&self.0).output().unwrap()
    }
    fn glance(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_glance")).args(args).current_dir(&self.0).output().unwrap()
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn git_commands_go_to_git() {
    let repo = Repo::new("commit");
    std::fs::write(repo.path().join("a.txt"), "a\n").unwrap();
    // glance's own command (piped, so it doesn't open a screen).
    assert!(repo.glance(&["add", "a.txt"]).status.success());
    let out = repo.glance(&["commit", "-q", "-m", "first"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let log = repo.git(&["log", "--format=%s"]);
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "first");
    // An option glance's log doesn't have: git's log.
    let out = repo.glance(&["log", "--format=%s"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "first");
}

#[test]
fn git_exit_codes_and_errors_come_through() {
    let repo = Repo::new("codes");
    let out = repo.glance(&["rev-parse", "--verify", "nope"]);
    assert_eq!(out.status.code(), Some(128));
    let out = repo.glance(&["pussh"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is not a git command"));
    // A mistake on a glance-only command is glance's.
    let out = repo.glance(&["branches", "--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--bogus"));
}
