//! Worktrees: the folders a repo is checked out in (`git worktree list`).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use gix::ObjectId;

use super::run;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    /// `None` on a bare repo or an unborn branch.
    pub head: Option<ObjectId>,
    /// Branch checked out; `None` when detached.
    pub branch: Option<String>,
    pub bare: bool,
    /// The repo's own folder; it can't be removed.
    pub main: bool,
    /// Locked, with the reason given (may be empty).
    pub locked: Option<String>,
    /// Its folder is gone; `git worktree prune` cleans it up.
    pub prunable: Option<String>,
}

/// Every worktree, the main one first.
pub fn list(repo: &gix::Repository) -> Result<Vec<Worktree>> {
    let dir = repo.workdir().unwrap_or(repo.git_dir());
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .context("could not run git")?;
    if !out.status.success() {
        bail!("git worktree list: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(parse(&String::from_utf8_lossy(&out.stdout)))
}

/// Reads `git worktree list --porcelain`: one block per worktree, blank line between.
pub fn parse(text: &str) -> Vec<Worktree> {
    let mut out: Vec<Worktree> = Vec::new();
    for line in text.lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if key == "worktree" {
            out.push(Worktree { path: PathBuf::from(value), main: out.is_empty(), ..Default::default() });
            continue;
        }
        let Some(w) = out.last_mut() else { continue };
        match key {
            "HEAD" => w.head = ObjectId::from_hex(value.as_bytes()).ok().filter(|id| !id.is_null()),
            "branch" => w.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).to_string()),
            "bare" => w.bare = true,
            "locked" => w.locked = Some(value.to_string()),
            "prunable" => w.prunable = Some(value.to_string()),
            _ => {}
        }
    }
    out
}

/// Where `gg wt` puts a new worktree: next to the main one, named `<repo>-<branch>`.
pub fn suggest_path(main: &Path, branch: &str) -> PathBuf {
    let name = main.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "repo".into());
    let slug: String = branch.chars().map(|c| if c == '/' || c.is_whitespace() { '-' } else { c }).collect();
    main.parent().unwrap_or(main).join(format!("{name}-{slug}"))
}

// ---------------------------------------------------------------- writes

/// Where a new worktree's branch comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// A local branch of that name: checked out there.
    Local,
    /// Only on a remote: git makes a local branch tracking it.
    Remote,
    /// Nowhere yet: a new branch from HEAD.
    New,
}

pub fn source(repo: &gix::Repository, branch: &str) -> Source {
    let has = |name: String| repo.find_reference(name.as_str()).is_ok();
    if has(format!("refs/heads/{branch}")) {
        Source::Local
    } else if repo.remote_names().iter().any(|r| has(format!("refs/remotes/{r}/{branch}"))) {
        Source::Remote
    } else {
        Source::New
    }
}

pub fn add_args(path: &Path, branch: &str, from: Source) -> Vec<String> {
    let dir = path.display().to_string();
    match from {
        Source::Local | Source::Remote => run::args(&["worktree", "add", &dir, branch]),
        Source::New => run::args(&["worktree", "add", "-b", branch, &dir]),
    }
}

pub fn add(path: &Path, branch: &str, from: Source) -> (bool, String) {
    run::git(&add_args(path, branch, from))
}

/// `force` also throws away uncommitted and untracked files in it.
pub fn remove(path: &Path, force: bool) -> (bool, String) {
    let dir = path.display().to_string();
    let mut a = run::args(&["worktree", "remove"]);
    if force {
        a.push("--force".into());
    }
    a.push(dir);
    run::git(&a)
}

/// Forgets worktrees whose folders are gone.
pub fn prune() -> (bool, String) {
    run::git(&run::args(&["worktree", "prune"]))
}

/// `git worktree lock` (or `unlock`): a locked one isn't pruned or removed.
pub fn lock(path: &Path, on: bool) -> (bool, String) {
    run::git(&run::args(&["worktree", if on { "lock" } else { "unlock" }, &path.display().to_string()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain() {
        let text = "worktree /r/app\nHEAD 84c6ea301e84528ac3b52730b2a4242042933572\nbranch refs/heads/main\n\n\
                    worktree /tmp/x\nHEAD d5a61cfe0000000000000000000000000000000a\ndetached\nprunable gitdir file points to non-existent location\n\n\
                    worktree /r/app-fix\nHEAD 0bacb9f50000000000000000000000000000000b\nbranch refs/heads/fix/parser\nlocked\n\n";
        let list = parse(text);
        assert_eq!(list.len(), 3);
        assert!(list[0].main && !list[1].main);
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert_eq!(list[1].branch, None);
        assert!(list[1].prunable.as_deref().is_some_and(|r| r.starts_with("gitdir")));
        assert_eq!(list[2].branch.as_deref(), Some("fix/parser"));
        assert_eq!(list[2].locked.as_deref(), Some(""));
    }

    #[test]
    fn suggests_paths() {
        assert_eq!(suggest_path(Path::new("/r/app"), "fix/parser"), PathBuf::from("/r/app-fix-parser"));
    }

    #[test]
    fn add_checks_out_or_creates() {
        let p = Path::new("/r/app-x");
        assert_eq!(add_args(p, "x", Source::Local), ["worktree", "add", "/r/app-x", "x"]);
        assert_eq!(add_args(p, "x", Source::Remote), ["worktree", "add", "/r/app-x", "x"]);
        assert_eq!(add_args(p, "x", Source::New), ["worktree", "add", "-b", "x", "/r/app-x"]);
    }

    /// A throwaway repo with one commit, a local branch `here` and a remote-only `there`.
    #[test]
    fn picks_the_branch_source() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("gg-test-wt-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q"]);
        git(&["commit", "-q", "--allow-empty", "-m", "first"]);
        git(&["branch", "here"]);
        git(&["remote", "add", "origin", "https://example.invalid/r.git"]);
        git(&["update-ref", "refs/remotes/origin/there", "HEAD"]);
        let repo = gix::open(&dir).unwrap();
        assert_eq!(source(&repo, "here"), Source::Local);
        assert_eq!(source(&repo, "there"), Source::Remote);
        assert_eq!(source(&repo, "nowhere"), Source::New);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
