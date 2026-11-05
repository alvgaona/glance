use anyhow::Result;
use gix::ObjectId;
use gix::bstr::ByteSlice;

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
}

impl Branch {
    /// The upstream was deleted on the remote.
    pub fn gone(&self) -> bool {
        self.upstream.as_ref().is_some_and(|u| u.name.ends_with("(gone)"))
    }

    /// `(remote, branch)` to delete on the remote: the upstream of a local branch,
    /// or the branch itself when it is a remote one. `None` if there's nothing there.
    pub fn remote_target(&self) -> Option<(String, String)> {
        let full = if self.remote {
            self.name.clone()
        } else {
            let up = self.upstream.as_ref().filter(|_| !self.gone())?;
            up.name.clone()
        };
        let (remote, branch) = full.split_once('/')?;
        Some((remote.to_string(), branch.to_string()))
    }
}

pub struct Branches {
    /// `main`, `master` or whatever `origin/HEAD` points to, and its tip, if found.
    pub default: Option<(String, ObjectId)>,
    /// Newest first.
    pub list: Vec<Branch>,
}

/// Local branches, plus remote ones with `remotes` (skipping `origin/HEAD`).
pub fn read(repo: &gix::Repository, remotes: bool) -> Result<Branches> {
    let current = repo.head_name()?.map(|n| n.shorten().to_string());
    let default = default_branch(repo);

    let refs = repo.references()?;
    let mut iters = vec![refs.local_branches()?];
    if remotes {
        iters.push(refs.remote_branches()?);
    }

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

            let merged = match &default {
                // Same tip as main counts too (fast-forward merges); main and origin/main don't.
                Some((def_name, def_id)) if *def_name != name && !name.ends_with(&format!("/{def_name}")) => {
                    tip == *def_id || repo.rev_walk([tip]).with_hidden([*def_id]).all()?.next().is_none()
                }
                _ => false,
            };

            list.push(Branch {
                current: !remote && current.as_deref() == Some(name.as_str()),
                remote,
                oid: tip,
                id: commit.id().shorten_or_id().to_string(),
                time: commit.time()?.seconds,
                summary: commit.message()?.summary().to_str_lossy().into_owned(),
                author: commit.author()?.name.to_str_lossy().into_owned(),
                upstream: if remote { None } else { upstream(repo, full.as_ref(), tip).ok().flatten() },
                merged,
                name,
            });
        }
    }
    list.sort_by_key(|b| std::cmp::Reverse(b.time));
    Ok(Branches { default, list })
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
        let files = super::show::tree_changes(repo, &base.tree()?, &tip.tree()?)?;
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
    for name in candidates {
        if let Ok(mut r) = repo.find_reference(format!("refs/heads/{name}").as_str())
            && let Ok(id) = r.peel_to_id()
        {
            return Some((name, id.detach()));
        }
    }
    None
}
