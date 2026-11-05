//! What `gg rebase` needs to know about the repo: where a branch split off, what isn't
//! pushed, and how far a rebase in progress has got.

use anyhow::Result;
use gix::ObjectId;

use super::todo;

/// Where this branch split off the default branch, and a description like
/// "4 commits since branching off main". Compares with both the local and origin
/// copies of main and keeps the closer one. `None` if there's nothing to rebase.
pub fn fork_point(repo: &gix::Repository) -> Result<Option<(ObjectId, String)>> {
    let Ok(head) = repo.head_id() else {
        return Ok(None);
    };
    let head = head.detach();
    let Some((main, _)) = super::branches::default_branch(repo) else {
        return Ok(None);
    };
    // On main itself there's no fork; `unpushed` covers that case.
    if repo.head_name()?.is_some_and(|n| n.shorten() == main.as_str()) {
        return Ok(None);
    }

    let mut best: Option<(usize, ObjectId, String)> = None;
    for name in [format!("refs/heads/{main}"), format!("refs/remotes/origin/{main}")] {
        let Ok(mut r) = repo.find_reference(name.as_str()) else {
            continue;
        };
        let short = r.name().shorten().to_string();
        let Ok(tip) = r.peel_to_id() else { continue };
        let Ok(Some(base)) = repo.merge_base(head, tip.detach()) else {
            continue;
        };
        let base = base.detach();
        let n = repo.rev_walk([head]).with_hidden([base]).all()?.count();
        if n > 0 && best.as_ref().is_none_or(|(m, ..)| n < *m) {
            best = Some((n, base, short));
        }
    }
    Ok(best.map(|(n, base, from)| {
        let s = if n == 1 { "" } else { "s" };
        (base, format!("{n} commit{s} since branching off {from}"))
    }))
}

/// Commits not yet pushed to this branch's upstream, e.g. on main before a push.
pub fn unpushed(repo: &gix::Repository) -> Result<Option<(ObjectId, String)>> {
    let (Some(name), Ok(head)) = (repo.head_name()?, repo.head_id()) else {
        return Ok(None);
    };
    let Some(Ok(up)) = repo.branch_remote_tracking_ref_name(name.as_ref(), gix::remote::Direction::Fetch) else {
        return Ok(None);
    };
    let Ok(mut up_ref) = repo.find_reference(up.as_ref()) else {
        return Ok(None);
    };
    let head = head.detach();
    let Ok(Some(base)) = repo.merge_base(head, up_ref.peel_to_id()?.detach()) else {
        return Ok(None);
    };
    let base = base.detach();
    let n = repo.rev_walk([head]).with_hidden([base]).all()?.count();
    if n == 0 {
        return Ok(None);
    }
    let s = if n == 1 { "" } else { "s" };
    Ok(Some((base, format!("{n} unpushed commit{s} (not on {})", up.shorten()))))
}

/// A rebase in progress, from git's `rebase-merge` folder.
pub struct Progress {
    /// Branch being rebased ("HEAD" when detached).
    pub branch: String,
    /// Short id of the commit it goes onto, if known.
    pub onto: String,
    /// Steps already applied.
    pub done: usize,
}

pub fn progress(repo: &gix::Repository) -> Progress {
    let dir = repo.path().join("rebase-merge");
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string());
    Progress {
        branch: read("head-name")
            .map(|h| h.trim_start_matches("refs/heads/").to_string())
            .unwrap_or_else(|| "HEAD".into()),
        onto: read("onto").map(|o| o.chars().take(7).collect()).unwrap_or_default(),
        done: read("done").map(|d| todo::parse(&d).len()).unwrap_or(0),
    }
}

/// `git config gg.rebaseOrder oldest` shows git's order; anything else (or unset) shows newest first.
pub fn newest_first(repo: &gix::Repository) -> bool {
    let order = repo.config_snapshot().string("gg.rebaseOrder");
    !order.is_some_and(|o| o.eq_ignore_ascii_case(b"oldest"))
}
