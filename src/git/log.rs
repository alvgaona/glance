use std::collections::HashMap;

use anyhow::Result;
use gix::ObjectId;
use gix::bstr::ByteSlice;
use gix::traverse::commit::topo;

#[derive(Debug)]
pub struct Commit {
    pub oid: ObjectId,
    pub id: String,
    pub parents: Vec<ObjectId>,
    pub summary: String,
    pub author: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub is_merge: bool,
    /// Branch and tag names pointing at this commit.
    pub refs: Vec<RefLabel>,
}

#[derive(Debug, Clone)]
pub enum RefLabel {
    Head(String),
    Branch(String),
    Remote(String),
    Tag(String),
}

/// Newest first, children always before parents (like `git log --graph`).
/// With `all`, walks from every branch, remote and tag instead of just HEAD.
pub fn read(repo: &gix::Repository, count: usize, all: bool) -> Result<Vec<Commit>> {
    let labels = ref_labels(repo)?;
    let mut tips: Vec<ObjectId> = repo.head_id().ok().map(|id| id.detach()).into_iter().collect();
    if all {
        tips.extend(labels.keys().copied());
    }
    if tips.is_empty() {
        return Ok(Vec::new()); // unborn branch: no commits yet
    }

    let walk = topo::Builder::new(&repo.objects)
        .with_tips(tips)
        // Keeps each branch's commits together, like `git log --graph`.
        .sorting(topo::Sorting::TopoOrder)
        .build()?;

    walk.take(count)
        .map(|info| {
            let info = info?;
            let commit = repo.find_commit(info.id)?;
            let author = commit.author()?;
            let parents: Vec<ObjectId> = commit.parent_ids().map(|id| id.detach()).collect();
            Ok(Commit {
                oid: info.id,
                id: commit.id().shorten_or_id().to_string(),
                is_merge: parents.len() > 1,
                parents,
                summary: commit.message()?.summary().to_str_lossy().into_owned(),
                author: author.name.to_str_lossy().into_owned(),
                time: commit.time()?.seconds,
                refs: labels.get(&info.id).cloned().unwrap_or_default(),
            })
        })
        .collect()
}

/// Branch, remote and tag names for every commit that has one.
pub fn ref_labels(repo: &gix::Repository) -> Result<HashMap<ObjectId, Vec<RefLabel>>> {
    let mut map: HashMap<ObjectId, Vec<RefLabel>> = HashMap::new();
    let head_branch = repo.head_name()?.map(|n| n.shorten().to_string());

    let platform = repo.references()?;
    for r in platform.all()? {
        let Ok(mut r) = r else { continue };
        // Read the name first: peeling a symbolic ref (origin/HEAD) renames it to its target.
        let full = r.name().as_bstr().to_str_lossy().into_owned();
        let short = r.name().shorten().to_string();
        let Ok(id) = r.peel_to_id() else { continue };
        let id = id.detach();
        let label = if full.starts_with("refs/heads/") {
            if head_branch.as_deref() == Some(short.as_str()) { RefLabel::Head(short) } else { RefLabel::Branch(short) }
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
