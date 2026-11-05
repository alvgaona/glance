//! Pull requests from GitHub through the `gh` CLI. Slow (seconds), so callers run it in the background.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use gix::ObjectId;
use gix::bstr::BStr;
use serde::Deserialize;

use crate::git::branches::Branch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Default)]
pub struct Checks {
    pub passed: usize,
    pub failed: usize,
    pub pending: usize,
}

#[derive(Debug, Clone)]
pub struct Pr {
    pub number: u64,
    pub state: State,
    pub title: String,
    pub url: String,
    pub branch: String,
    pub head: Option<ObjectId>,
    /// Login of the account whose repo the branch lives in.
    pub owner: String,
    /// "APPROVED", "CHANGES_REQUESTED", "REVIEW_REQUIRED", or None.
    pub review: Option<String>,
    /// Only fetched for open PRs.
    pub checks: Option<Checks>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPr {
    number: u64,
    state: String,
    is_draft: bool,
    title: String,
    url: String,
    head_ref_name: String,
    head_ref_oid: String,
    head_repository_owner: Option<RawOwner>,
    review_decision: Option<String>,
}

#[derive(Deserialize)]
struct RawOwner {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawChecks {
    number: u64,
    status_check_rollup: Option<Vec<RawCheck>>,
}

/// A CheckRun (status + conclusion) or a StatusContext (state).
#[derive(Deserialize)]
struct RawCheck {
    status: Option<String>,
    conclusion: Option<String>,
    state: Option<String>,
}

/// True when the repo has a GitHub remote and `gh` is installed.
pub fn available(repo: &gix::Repository) -> bool {
    let on_github = repo.remote_names().into_iter().any(|name| {
        repo.find_remote(AsRef::<BStr>::as_ref(&name))
            .ok()
            .and_then(|r| r.url(gix::remote::Direction::Fetch).map(|u| u.to_bstring().to_string()))
            .is_some_and(|u| u.contains("github.com"))
    });
    on_github && crate::process::which("gh")
}

/// The last 100 PRs (any state), with CI checks for the open ones. Runs both `gh` calls at once.
pub fn fetch(dir: &Path) -> Result<Vec<Pr>> {
    let run = |args: &[&str]| -> Result<String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        // Nothing on stdin: gh must never wait for the terminal behind the full screen.
        let out = crate::process::run_with_input("gh", &args, Some(dir), b"", &[]).context("could not run gh")?;
        if !out.ok() {
            bail!("{}", out.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("gh failed").trim());
        }
        Ok(out.stdout)
    };
    let fields = "number,state,isDraft,title,url,headRefName,headRefOid,headRepositoryOwner,reviewDecision";
    let (list, checks) = std::thread::scope(|s| {
        let checks =
            s.spawn(|| run(&["pr", "list", "--state", "open", "--limit", "30", "--json", "number,statusCheckRollup"]));
        let list = run(&["pr", "list", "--state", "all", "--limit", "100", "--json", fields]);
        (list, checks.join().unwrap_or_else(|_| bail!("gh crashed")))
    });

    let raw: Vec<RawPr> = serde_json::from_str(&list?).context("unexpected output from gh")?;
    // Checks are a bonus: without them the list is still useful.
    let checks: HashMap<u64, Checks> = checks
        .ok()
        .and_then(|bytes| serde_json::from_str::<Vec<RawChecks>>(&bytes).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.number, summarize(c.status_check_rollup.unwrap_or_default())))
        .collect();

    Ok(raw
        .into_iter()
        .map(|p| Pr {
            state: match (p.state.as_str(), p.is_draft) {
                ("MERGED", _) => State::Merged,
                ("CLOSED", _) => State::Closed,
                (_, true) => State::Draft,
                _ => State::Open,
            },
            checks: checks.get(&p.number).cloned(),
            number: p.number,
            title: p.title,
            url: p.url,
            branch: p.head_ref_name,
            head: ObjectId::from_hex(p.head_ref_oid.as_bytes()).ok(),
            owner: p.head_repository_owner.map(|o| o.login).unwrap_or_default(),
            review: p.review_decision.filter(|r| !r.is_empty()),
        })
        .collect())
}

fn summarize(list: Vec<RawCheck>) -> Checks {
    let mut c = Checks::default();
    for check in list {
        let outcome = check.conclusion.or(check.state).unwrap_or_default();
        let done = check.status.as_deref().is_none_or(|s| s == "COMPLETED");
        match outcome.as_str() {
            _ if !done => c.pending += 1,
            "PENDING" | "EXPECTED" | "" => c.pending += 1,
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => c.failed += 1,
            _ => c.passed += 1,
        }
    }
    c
}

/// `alvgaona` from `https://github.com/alvgaona/x.git` or `git@github.com:alvgaona/x`,
/// also through an ssh host alias like `github.com-work`.
fn owner_of(url: &str) -> Option<String> {
    let rest = url.split("github.com").nth(1)?;
    let rest = &rest[rest.find([':', '/'])?..];
    let owner = rest.trim_start_matches([':', '/']).split('/').next()?;
    (!owner.is_empty()).then(|| owner.to_lowercase())
}

/// Remote name -> GitHub owner, for matching PRs to branches.
pub fn remote_owners(repo: &gix::Repository) -> HashMap<String, String> {
    repo.remote_names()
        .into_iter()
        .filter_map(|name| {
            let r = repo.find_remote(AsRef::<BStr>::as_ref(&name)).ok()?;
            let url = r.url(gix::remote::Direction::Fetch)?.to_bstring().to_string();
            Some((name.to_string(), owner_of(&url)?))
        })
        .collect()
}

/// The PR for a local branch: same branch name, and either its head is the branch's tip
/// (here or on a remote) or it comes from the account of a remote the branch is on.
/// Open PRs win over closed ones; then the newest.
/// PRs from a repo's main branch (`main` of some fork) never count: a feature branch
/// tracking `upstream/main` isn't that PR.
pub fn find<'a>(prs: &'a [Pr], b: &Branch, owners: &HashMap<String, String>, main: Option<&str>) -> Option<&'a Pr> {
    if b.remote || Some(b.name.as_str()) == main {
        return None;
    }
    let tips: Vec<ObjectId> = std::iter::once(b.oid).chain(b.copies.iter().map(|c| c.oid)).collect();
    let mut accounts: Vec<&String> = b.copies.iter().filter_map(|c| owners.get(&c.remote)).collect();
    if accounts.is_empty() {
        // Never pushed: it would be pushed to origin.
        accounts.extend(owners.get("origin"));
    }
    let names: Vec<&str> = std::iter::once(b.name.as_str())
        .chain(b.copies.iter().map(|c| c.branch.as_str()))
        .filter(|n| Some(*n) != main && !["main", "master"].contains(n))
        .collect();
    let mut found: Vec<&Pr> = prs
        .iter()
        .filter(|p| names.contains(&p.branch.as_str()))
        .filter(|p| {
            p.head.is_some_and(|h| tips.contains(&h)) || accounts.iter().any(|a| a.eq_ignore_ascii_case(&p.owner))
        })
        .collect();
    // gh lists newest first; a stable sort keeps that order within each group.
    found.sort_by_key(|p| matches!(p.state, State::Merged | State::Closed));
    found.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners() {
        assert_eq!(owner_of("https://github.com/alvgaona/glance.git").as_deref(), Some("alvgaona"));
        assert_eq!(owner_of("git@github.com:Aerostack2/x").as_deref(), Some("aerostack2"));
        assert_eq!(owner_of("ssh://git@github.com/a/b").as_deref(), Some("a"));
        assert_eq!(owner_of("https://gitlab.com/a/b"), None);
        assert_eq!(owner_of("git@github.com-work:alvgaona/x").as_deref(), Some("alvgaona"));
    }

    fn oid(n: u8) -> ObjectId {
        ObjectId::from_bytes_or_panic(&[n; 20])
    }

    fn pr(number: u64, state: State, branch: &str, head: u8, owner: &str) -> Pr {
        Pr {
            number,
            state,
            title: String::new(),
            url: String::new(),
            branch: branch.into(),
            head: Some(oid(head)),
            owner: owner.into(),
            review: None,
            checks: None,
        }
    }

    fn branch(name: &str, tip: u8, copies: &[(&str, &str, u8)]) -> Branch {
        Branch {
            name: name.into(),
            remote: false,
            current: false,
            oid: oid(tip),
            id: String::new(),
            time: 0,
            summary: String::new(),
            author: String::new(),
            upstream: None,
            merged: false,
            copies: copies
                .iter()
                .map(|(r, b, o)| crate::git::branches::Copy { remote: (*r).into(), branch: (*b).into(), oid: oid(*o) })
                .collect(),
            worktree: None,
        }
    }

    #[test]
    fn matching() {
        let owners: HashMap<String, String> =
            [("origin".to_string(), "me".to_string()), ("upstream".to_string(), "org".to_string())].into();
        let prs = vec![
            pr(5, State::Open, "feat", 9, "someone"), // same name, other fork, other commit
            pr(4, State::Merged, "feat", 1, "me"),    // squash-merged, same tip
            pr(3, State::Merged, "main", 7, "org"),   // the org's own main
            pr(2, State::Open, "fix", 8, "stranger"), // other fork, but its head is our tip
            pr(1, State::Closed, "solo", 6, "me"),
        ];
        let main = Some("main");
        let num = |b: &Branch| find(&prs, b, &owners, main).map(|p| p.number);

        // Our PR, not the stranger's same-named one.
        assert_eq!(num(&branch("feat", 1, &[("origin", "feat", 1)])), Some(4));
        // Tracking upstream/main doesn't make it PR #3.
        assert_eq!(num(&branch("topic", 7, &[("upstream", "main", 7)])), None);
        assert_eq!(num(&branch("main", 7, &[("upstream", "main", 7)])), None);
        // Head commit matches even from another account.
        assert_eq!(num(&branch("fix", 8, &[])), Some(2));
        // Never pushed: falls back to origin's owner.
        assert_eq!(num(&branch("solo", 3, &[])), Some(1));
    }

    #[test]
    fn open_wins_over_merged() {
        let owners: HashMap<String, String> = [("origin".to_string(), "me".to_string())].into();
        let prs = vec![pr(2, State::Merged, "feat", 1, "me"), pr(1, State::Open, "feat", 2, "me")];
        let b = branch("feat", 2, &[("origin", "feat", 2)]);
        assert_eq!(find(&prs, &b, &owners, Some("main")).map(|p| p.number), Some(1));
    }

    #[test]
    fn checks() {
        let c = |status: &str, conclusion: &str| RawCheck {
            status: Some(status.into()),
            conclusion: Some(conclusion.into()),
            state: None,
        };
        let s = summarize(vec![
            c("COMPLETED", "SUCCESS"),
            c("COMPLETED", "SKIPPED"),
            c("COMPLETED", "FAILURE"),
            c("IN_PROGRESS", ""),
            RawCheck { status: None, conclusion: None, state: Some("PENDING".into()) },
        ]);
        assert_eq!((s.passed, s.failed, s.pending), (2, 1, 2));
    }
}
