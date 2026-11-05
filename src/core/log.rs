use std::collections::HashMap;

use anyhow::Result;
use gix::bstr::ByteSlice;

#[derive(Debug)]
pub struct Commit {
    pub id: String,
    pub summary: String,
    pub author: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub is_merge: bool,
    /// Branch and tag names pointing at this commit.
    pub refs: Vec<RefLabel>,
}

#[derive(Debug)]
pub enum RefLabel {
    Head(String),
    Branch(String),
    Remote(String),
    Tag(String),
}

/// Newest first. With `all`, walks from every branch, remote and tag instead of just HEAD.
pub fn read(repo: &gix::Repository, count: usize, all: bool) -> Result<Vec<Commit>> {
    let labels = ref_labels(repo)?;
    let mut tips: Vec<gix::ObjectId> = repo.head_id().ok().map(|id| id.detach()).into_iter().collect();
    if all {
        tips.extend(labels.keys().copied());
    }
    if tips.is_empty() {
        return Ok(Vec::new()); // unborn branch: no commits yet
    }

    let walk = repo
        .rev_walk(tips)
        .sorting(gix::revision::walk::Sorting::ByCommitTime(Default::default()))
        .all()?;
    let mut out = Vec::with_capacity(count.min(1024));
    for info in walk.take(count) {
        let info = info?;
        let commit = info.object()?;
        let author = commit.author()?;
        out.push(Commit {
            id: info.id().shorten_or_id().to_string(),
            summary: commit.message()?.summary().to_str_lossy().into_owned(),
            author: author.name.to_str_lossy().into_owned(),
            time: commit.time()?.seconds,
            is_merge: commit.parent_ids().count() > 1,
            refs: labels.get(&info.id).map(|v| v.iter().map(clone_label).collect()).unwrap_or_default(),
        });
    }
    Ok(out)
}

fn clone_label(l: &RefLabel) -> RefLabel {
    match l {
        RefLabel::Head(s) => RefLabel::Head(s.clone()),
        RefLabel::Branch(s) => RefLabel::Branch(s.clone()),
        RefLabel::Remote(s) => RefLabel::Remote(s.clone()),
        RefLabel::Tag(s) => RefLabel::Tag(s.clone()),
    }
}

fn ref_labels(repo: &gix::Repository) -> Result<HashMap<gix::ObjectId, Vec<RefLabel>>> {
    let mut map: HashMap<gix::ObjectId, Vec<RefLabel>> = HashMap::new();
    let head_branch = repo.head_name()?.map(|n| n.shorten().to_string());

    let platform = repo.references()?;
    for r in platform.all()? {
        let Ok(mut r) = r else { continue };
        let Ok(id) = r.peel_to_id() else { continue };
        let id = id.detach();
        let full = r.name().as_bstr().to_str_lossy().into_owned();
        let short = r.name().shorten().to_string();
        let label = if let Some(_) = full.strip_prefix("refs/heads/") {
            if head_branch.as_deref() == Some(short.as_str()) {
                RefLabel::Head(short)
            } else {
                RefLabel::Branch(short)
            }
        } else if full.starts_with("refs/remotes/") {
            if short.ends_with("/HEAD") {
                continue;
            }
            RefLabel::Remote(short)
        } else if full.starts_with("refs/tags/") {
            RefLabel::Tag(short)
        } else {
            continue;
        };
        map.entry(id).or_default().push(label);
    }
    Ok(map)
}
