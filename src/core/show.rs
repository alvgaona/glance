use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use gix::object::tree::diff::ChangeDetached;

use super::diff::{self, Size};
use super::log::{RefLabel, ref_labels};
use super::status::ChangeKind;

#[derive(Debug)]
pub struct Shown {
    pub oid: gix::ObjectId,
    pub id: String,
    pub refs: Vec<RefLabel>,
    pub author: String,
    pub email: String,
    /// Seconds since the Unix epoch, and the author's UTC offset in seconds.
    pub time: i64,
    pub offset: i32,
    pub subject: String,
    pub body: String,
    pub parents: Vec<String>,
    pub files: Vec<FileStat>,
}

#[derive(Debug)]
pub struct FileStat {
    pub kind: ChangeKind,
    pub path: String,
    pub from: Option<String>,
    pub size: Option<Size>,
    /// Blob before and after, for building the diff later.
    pub old: Option<gix::ObjectId>,
    pub new: Option<gix::ObjectId>,
}

/// Unified diff of one file of a shown commit.
pub fn file_patch(repo: &gix::Repository, f: &FileStat) -> Option<Vec<diff::PatchLine>> {
    let read = |id: Option<gix::ObjectId>| id.map(|id| diff::blob(repo, id)).unwrap_or_default();
    diff::patch(&read(f.old), &read(f.new), 3)
}

/// One commit and what it changed compared with its first parent.
pub fn read(repo: &gix::Repository, rev: &str) -> Result<Shown> {
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
    let changes = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)?;

    let mut files = Vec::new();
    for change in changes {
        let (kind, path, from, old, new, mode) = match change {
            ChangeDetached::Addition { location, id, entry_mode, .. } => (ChangeKind::Added, location, None, None, Some(id), entry_mode),
            ChangeDetached::Deletion { location, id, entry_mode, .. } => (ChangeKind::Deleted, location, None, Some(id), None, entry_mode),
            ChangeDetached::Modification {
                location,
                previous_id,
                id,
                entry_mode,
                ..
            } => (ChangeKind::Modified, location, None, Some(previous_id), Some(id), entry_mode),
            ChangeDetached::Rewrite {
                source_location,
                source_id,
                location,
                id,
                copy,
                entry_mode,
                ..
            } => {
                let kind = if copy { ChangeKind::Copied } else { ChangeKind::Renamed };
                (kind, location, Some(source_location), Some(source_id), Some(id), entry_mode)
            }
        };
        if mode.is_tree() {
            continue;
        }
        let read = |id: Option<gix::ObjectId>| id.map(|id| diff::blob(repo, id)).unwrap_or_default();
        files.push(FileStat {
            kind,
            path: path.to_str_lossy().into_owned(),
            from: from.map(|f| f.to_str_lossy().into_owned()),
            size: if mode.is_commit() { None } else { diff::count(&read(old), &read(new)) },
            old: old.filter(|_| !mode.is_commit()),
            new: new.filter(|_| !mode.is_commit()),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let time = commit.time()?;
    Ok(Shown {
        oid: commit.id,
        id: commit.id().shorten_or_id().to_string(),
        refs: ref_labels(repo)?.remove(&commit.id).unwrap_or_default(),
        author: author.name.to_str_lossy().into_owned(),
        email: author.email.to_str_lossy().into_owned(),
        time: time.seconds,
        offset: time.offset,
        subject: message.summary().to_str_lossy().into_owned(),
        body: message.body().map(|b| b.to_str_lossy().trim().to_string()).unwrap_or_default(),
        parents: parents
            .iter()
            .map(|p| p.to_hex_with_len(id.shorten_or_id().hex_len()).to_string())
            .collect(),
        files,
    })
}
