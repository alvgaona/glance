use std::collections::HashSet;

use anyhow::Result;
use gix::ObjectId;
use gix::bstr::ByteSlice;

use super::run::{self, Ran};
use super::status::{Upstream, upstream};

#[derive(Debug, Clone)]
pub struct Branch {
    /// Short name: `feat/x`, or `origin/feat/x` for a remote branch.
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub oid: ObjectId,
    pub id: String,
    /// Commit time of the tip, seconds since the Unix epoch.
    pub time: i64,
    pub summary: String,
    pub author: String,
    pub upstream: Option<Upstream>,
    /// Fully contained in the default branch (safe to delete).
    pub merged: bool,
    /// For a local branch: the remotes that have it (same name, or its upstream), as last fetched.
    pub copies: Vec<Copy>,
    /// Checked out (or being rebased) in another worktree, at this path.
    /// Git won't delete it or check it out here while that lasts.
    pub worktree: Option<std::path::PathBuf>,
}

/// A local branch as it exists on one remote.
#[derive(Debug, Clone)]
pub struct Copy {
    pub remote: String,
    /// Branch name on the remote (usually the same as the local one).
    pub branch: String,
    pub oid: ObjectId,
}

impl Branch {
    /// The upstream was deleted on the remote.
    pub fn gone(&self) -> bool {
        self.upstream.as_ref().is_some_and(|u| u.name.ends_with("(gone)"))
    }

    /// `(remote, branch)` pairs to delete on remotes: every copy of a local branch,
    /// or the branch itself when it is a remote one.
    pub fn remote_targets(&self, remotes: &[String]) -> Vec<(String, String)> {
        if self.remote {
            return split_remote(remotes, &self.name)
                .map(|(r, b)| (r.to_string(), b.to_string()))
                .into_iter()
                .collect();
        }
        self.copies.iter().map(|c| (c.remote.clone(), c.branch.clone())).collect()
    }
}

/// `origin/feat` → `("origin", "feat")`. The remote is the longest remote name that
/// prefixes it: names may have a `/` (`my/fork/feat` is `feat` on `my/fork`).
pub fn split_remote<'a>(remotes: &[String], name: &'a str) -> Option<(&'a str, &'a str)> {
    let r = remotes
        .iter()
        .filter(|r| name.strip_prefix(r.as_str()).is_some_and(|b| b.starts_with('/')))
        .max_by_key(|r| r.len())?;
    Some((&name[..r.len()], &name[r.len() + 1..]))
}

/// Where `name` exists on each remote: `refs/remotes/<remote>/<name>`, plus the upstream
/// (which may have another name). Upstream first, then by remote name.
fn copies(repo: &gix::Repository, remotes: &[String], name: &str, upstream: Option<&Upstream>) -> Vec<Copy> {
    let mut out: Vec<Copy> = Vec::new();
    let mut add = |remote: &str, branch: &str| {
        if out.iter().any(|c| c.remote == remote && c.branch == branch) {
            return;
        }
        if let Ok(mut r) = repo.find_reference(format!("refs/remotes/{remote}/{branch}").as_str())
            && let Ok(id) = r.peel_to_id()
        {
            out.push(Copy { remote: remote.to_string(), branch: branch.to_string(), oid: id.detach() });
        }
    };
    if let Some((r, b)) = upstream.filter(|u| !u.name.ends_with("(gone)")).and_then(|u| split_remote(remotes, &u.name))
    {
        add(r, b);
    }
    for r in remotes {
        add(r, name);
    }
    out
}

/// Branches held by worktrees other than this one: branch name -> worktree path.
/// Includes the main worktree when we're in a linked one.
fn other_worktrees(repo: &gix::Repository) -> std::collections::HashMap<String, std::path::PathBuf> {
    use std::path::{Path, PathBuf};
    // The branch a worktree's git dir holds: its HEAD, or the branch under a rebase.
    let held = |git_dir: &Path| -> Vec<String> {
        ["HEAD", "rebase-merge/head-name", "rebase-apply/head-name"]
            .iter()
            .filter_map(|file| std::fs::read_to_string(git_dir.join(file)).ok())
            .filter_map(|s| {
                let s = s.trim();
                s.strip_prefix("ref: ").unwrap_or(s).strip_prefix("refs/heads/").map(str::to_owned)
            })
            .collect()
    };
    let mut dirs: Vec<(PathBuf, PathBuf)> = repo
        .worktrees()
        .into_iter()
        .flatten()
        .filter_map(|wt| Some((wt.git_dir().to_path_buf(), wt.base().ok()?)))
        .collect();
    let common = repo.common_dir();
    if repo.git_dir() != common
        && let Some(base) = common.parent()
    {
        dirs.push((common.to_path_buf(), base.to_path_buf()));
    }
    let here = repo.git_dir().canonicalize().ok();
    dirs.into_iter()
        .filter(|(git_dir, _)| git_dir.canonicalize().ok() != here)
        .flat_map(|(git_dir, base)| held(&git_dir).into_iter().map(move |b| (b, base.clone())))
        .collect()
}

/// How far `a` is ahead of and behind `b`.
pub fn ahead_behind(repo: &gix::Repository, a: ObjectId, b: ObjectId) -> (usize, usize) {
    if a == b {
        return (0, 0);
    }
    let count = |x: ObjectId, y: ObjectId| repo.rev_walk([x]).with_hidden([y]).all().map(|w| w.count()).unwrap_or(0);
    (count(a, b), count(b, a))
}

pub struct Branches {
    /// `main`, `master` or whatever `origin/HEAD` points to, and its tip, if found.
    pub default: Option<(String, ObjectId)>,
    /// Newest first.
    pub list: Vec<Branch>,
    /// Remote names, for splitting `origin/feat`.
    pub remotes: Vec<String>,
}

/// Local branches, plus remote ones with `remotes` (skipping `origin/HEAD`).
pub fn read(repo: &gix::Repository, remotes: bool) -> Result<Branches> {
    let current = repo.head_name()?.map(|n| n.shorten().to_string());
    let default = default_branch(repo);
    let remote_names: Vec<String> = repo.remote_names().into_iter().map(|r| r.to_string()).collect();
    let worktrees = other_worktrees(repo);

    let refs = repo.references()?;
    let mut iters = vec![refs.local_branches()?];
    if remotes {
        iters.push(refs.remote_branches()?);
    }

    // Everything in the default branch, walked once: a branch is merged when its tip is in it.
    let in_default: HashSet<ObjectId> = match &default {
        Some((_, id)) => repo.rev_walk([*id]).all()?.filter_map(Result::ok).map(|c| c.id).collect(),
        None => HashSet::new(),
    };

    let mut list = Vec::new();
    for (i, iter) in iters.into_iter().enumerate() {
        let remote = i == 1;
        for r in iter {
            let Ok(mut r) = r else { continue };
            let full = r.name().to_owned();
            let name = full.shorten().to_string();
            if remote && name.ends_with("/HEAD") {
                continue;
            }
            let Ok(tip) = r.peel_to_id() else { continue };
            let tip = tip.detach();
            let commit = repo.find_commit(tip)?;

            // Same tip as main counts too (fast-forward merges); main and origin/main don't.
            let is_default = match &default {
                Some((def, _)) if remote => split_remote(&remote_names, &name).is_some_and(|(_, b)| b == def),
                Some((def, _)) => *def == name,
                None => true,
            };
            let merged = !is_default && in_default.contains(&tip);

            let up = if remote { None } else { upstream(repo, full.as_ref(), tip).ok().flatten() };
            list.push(Branch {
                current: !remote && current.as_deref() == Some(name.as_str()),
                remote,
                oid: tip,
                id: commit.id().shorten_or_id().to_string(),
                time: commit.time()?.seconds,
                summary: commit.message()?.summary().to_str_lossy().into_owned(),
                author: commit.author()?.name.to_str_lossy().into_owned(),
                copies: if remote { Vec::new() } else { copies(repo, &remote_names, &name, up.as_ref()) },
                upstream: up,
                worktree: if remote { None } else { worktrees.get(&name).cloned() },
                merged,
                name,
            });
        }
    }
    list.sort_by_key(|b| std::cmp::Reverse(b.time));
    Ok(Branches { default, list, remotes: remote_names })
}

/// What a branch adds on top of the default branch.
pub struct Detail {
    /// Where it split off the default branch: short id and commit time.
    pub fork: Option<(String, i64)>,
    /// Commits not in the default branch, newest first (at most 50), and how many in total.
    pub unique: Vec<(String, String)>,
    pub unique_total: usize,
    /// Files, lines added and removed since the fork point.
    pub files: usize,
    pub added: u32,
    pub removed: u32,
}

pub fn detail(repo: &gix::Repository, b: &Branch, default: Option<ObjectId>) -> Result<Detail> {
    let mut d = Detail { fork: None, unique: Vec::new(), unique_total: 0, files: 0, added: 0, removed: 0 };
    let Some(def) = default else { return Ok(d) };
    let Ok(Some(base)) = repo.merge_base(b.oid, def) else {
        return Ok(d);
    };
    let base = repo.find_commit(base.detach())?;
    d.fork = Some((base.id().shorten_or_id().to_string(), base.time()?.seconds));

    for (n, info) in repo.rev_walk([b.oid]).with_hidden([def]).all()?.enumerate() {
        let info = info?;
        d.unique_total += 1;
        if n < 50 {
            let c = repo.find_commit(info.id)?;
            d.unique.push((c.id().shorten_or_id().to_string(), c.message()?.summary().to_str_lossy().into_owned()));
        }
    }

    if d.unique_total > 0 {
        let tip = repo.find_commit(b.oid)?;
        let files = super::tree::changes(repo, &base.tree()?, &tip.tree()?)?;
        d.files = files.len();
        for f in files {
            if let Some(super::diff::Size::Lines { added, removed }) = f.size {
                d.added += added;
                d.removed += removed;
            }
        }
    }
    Ok(d)
}

/// Commits that only this branch reaches: deleting it loses them (until gc).
/// Every other branch, remote branch and tag counts as "somewhere else".
pub fn only_here(repo: &gix::Repository, b: &Branch) -> usize {
    let own = if b.remote { format!("refs/remotes/{}", b.name) } else { format!("refs/heads/{}", b.name) };
    let Ok(refs) = repo.references() else {
        return 0;
    };
    let Ok(all) = refs.all() else { return 0 };
    let others: Vec<ObjectId> = all
        .filter_map(Result::ok)
        .filter(|r| r.name().as_bstr() != own.as_bytes())
        .filter_map(|mut r| r.peel_to_id().ok().map(|id| id.detach()))
        .collect();
    repo.rev_walk([b.oid]).with_hidden(others).all().map(|w| w.count()).unwrap_or(0)
}

/// Name and tip of the branch others are merged into.
pub fn default_branch(repo: &gix::Repository) -> Option<(String, ObjectId)> {
    // origin/HEAD -> origin/main -> local "main".
    let from_remote = repo
        .find_reference("refs/remotes/origin/HEAD")
        .ok()
        .and_then(|r| r.target().try_name().map(|n| n.shorten().to_string()))
        .and_then(|n| n.strip_prefix("origin/").map(str::to_owned));
    let candidates = from_remote.into_iter().chain(["main".to_string(), "master".to_string()]);
    candidates.into_iter().find_map(|name| {
        let id = repo.find_reference(format!("refs/heads/{name}").as_str()).ok()?.peel_to_id().ok()?.detach();
        Some((name, id))
    })
}

// ---------------------------------------------------------------- writes

pub fn switch(name: &str) -> Ran {
    run::git(&run::args(&["switch", name]))
}

pub fn rename(from: &str, to: &str) -> Ran {
    run::git(&run::args(&["branch", "-m", from, to]))
}

/// `git branch -D`: the screen has already warned about commits found nowhere else.
pub fn delete_args(name: &str) -> Vec<String> {
    run::args(&["branch", "-D", name])
}

/// Run in the terminal, not collected: a remote may ask for a password.
pub fn delete_remote_args(remote: &str, branch: &str) -> Vec<String> {
    run::args(&["push", remote, "--delete", branch])
}

/// `git fetch --all --prune` that never prompts (a prompt would draw over the screen):
/// a remote that needs a password fails and says so.
pub fn fetch(repo: &gix::Repository) -> impl FnOnce() -> Ran + Send + 'static {
    super::remote::no_prompt(repo, run::args(&["fetch", "--all", "--prune", "--quiet"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_args_name_the_branch() {
        assert_eq!(delete_args("feat/x"), ["branch", "-D", "feat/x"]);
        assert_eq!(delete_remote_args("origin", "feat/x"), ["push", "origin", "--delete", "feat/x"]);
    }

    #[test]
    fn splits_at_the_longest_remote_name() {
        let remotes = ["my".to_string(), "my/fork".to_string()];
        assert_eq!(split_remote(&remotes, "my/fork/feat"), Some(("my/fork", "feat")));
        assert_eq!(split_remote(&remotes, "my/feat"), Some(("my", "feat")));
        assert_eq!(split_remote(&remotes, "myx/feat"), None);
    }

    #[test]
    fn merged_branches_are_in_the_default_branch() {
        let s = crate::git::run::Scratch::new("branches-merged");
        s.commit("a.txt", "one\n");
        s.git(&["branch", "fix/main"]);
        s.git(&["branch", "ahead"]);
        s.git(&["checkout", "-q", "ahead"]);
        s.commit("b.txt", "two\n");
        s.git(&["checkout", "-q", "main"]);
        let b = read(&s.repo(), false).unwrap();
        let merged = |n: &str| b.list.iter().find(|x| x.name == n).unwrap().merged;
        // Ends in "/main", but it's a local branch, not the remote's main.
        assert!(merged("fix/main"));
        assert!(!merged("ahead"));
        assert!(!merged("main"));
    }
}
