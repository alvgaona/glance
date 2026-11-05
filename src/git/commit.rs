//! One commit in full: message, author, refs and the files it changed.

use anyhow::{Context, Result};
use gix::bstr::ByteSlice;

use super::log::{RefLabel, ref_labels};
use super::tree::{self, FileStat};

#[derive(Debug)]
pub struct Details {
    pub id: String,
    pub refs: Vec<RefLabel>,
    pub author: String,
    pub email: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub subject: String,
    pub body: String,
    pub parents: Vec<String>,
    pub files: Vec<FileStat>,
}

/// One commit and what it changed compared with its first parent.
pub fn read(repo: &gix::Repository, rev: &str) -> Result<Details> {
    let id = repo.rev_parse_single(rev).with_context(|| format!("unknown revision `{rev}`"))?;
    let commit = id.object()?.peel_to_commit().with_context(|| format!("`{rev}` is not a commit"))?;
    let author = commit.author()?;
    let message = commit.message()?;
    let parents: Vec<gix::ObjectId> = commit.parent_ids().map(|p| p.detach()).collect();

    let new_tree = commit.tree()?;
    let old_tree = match parents.first() {
        Some(p) => repo.find_commit(*p)?.tree()?,
        None => repo.empty_tree(),
    };
    let files = tree::changes(repo, &old_tree, &new_tree)?;

    Ok(Details {
        id: commit.id().shorten_or_id().to_string(),
        refs: ref_labels(repo)?.remove(&commit.id).unwrap_or_default(),
        author: author.name.to_str_lossy().into_owned(),
        email: author.email.to_str_lossy().into_owned(),
        time: commit.time()?.seconds,
        subject: message.summary().to_str_lossy().into_owned(),
        body: message.body().map(|b| b.to_str_lossy().trim().to_string()).unwrap_or_default(),
        parents: parents.iter().map(|p| p.to_hex_with_len(id.shorten_or_id().hex_len()).to_string()).collect(),
        files,
    })
}
