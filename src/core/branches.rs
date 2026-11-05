use anyhow::Result;
use gix::bstr::ByteSlice;

use super::status::{Upstream, upstream};

#[derive(Debug)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub id: String,
    /// Commit time of the tip, seconds since the Unix epoch.
    pub time: i64,
    pub summary: String,
    pub upstream: Option<Upstream>,
    /// Fully contained in the default branch (safe to delete).
    pub merged: bool,
}

pub struct Branches {
    /// `main`, `master` or whatever `origin/HEAD` points to, if found.
    pub default: Option<String>,
    /// Newest first.
    pub list: Vec<Branch>,
}

pub fn read(repo: &gix::Repository) -> Result<Branches> {
    let current = repo.head_name()?.map(|n| n.shorten().to_string());
    let default = default_branch(repo);

    let mut list = Vec::new();
    for r in repo.references()?.local_branches()? {
        let Ok(mut r) = r else { continue };
        let full = r.name().to_owned();
        let name = full.shorten().to_string();
        let Ok(tip) = r.peel_to_id() else { continue };
        let tip = tip.detach();
        let commit = repo.find_commit(tip)?;

        let merged = match &default {
            Some((def_name, def_id)) if *def_name != name => {
                repo.rev_walk([tip]).with_hidden([*def_id]).all()?.next().is_none()
            }
            _ => false,
        };

        list.push(Branch {
            current: current.as_deref() == Some(name.as_str()),
            id: commit.id().shorten_or_id().to_string(),
            time: commit.time()?.seconds,
            summary: commit.message()?.summary().to_str_lossy().into_owned(),
            upstream: upstream(repo, full.as_ref(), tip).ok().flatten(),
            merged,
            name,
        });
    }
    list.sort_by_key(|b| std::cmp::Reverse(b.time));
    Ok(Branches {
        default: default.map(|(n, _)| n),
        list,
    })
}

/// Name and tip of the branch others are merged into.
fn default_branch(repo: &gix::Repository) -> Option<(String, gix::ObjectId)> {
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
