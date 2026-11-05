//! Files that differ between two trees (a commit and its parent, a stash and its base).

use anyhow::Result;
use gix::bstr::ByteSlice;
use gix::object::tree::diff::ChangeDetached;

use super::diff::{self, Size};
use super::status::ChangeKind;

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

/// Unified diff of one changed file.
pub fn file_patch(repo: &gix::Repository, f: &FileStat) -> Option<Vec<diff::PatchLine>> {
    let read = |id: Option<gix::ObjectId>| id.map(|id| diff::blob(repo, id)).unwrap_or_default();
    diff::patch(&read(f.old), &read(f.new), 3)
}

/// Files that differ between two trees, with line counts, sorted by path.
pub fn changes(repo: &gix::Repository, old_tree: &gix::Tree<'_>, new_tree: &gix::Tree<'_>) -> Result<Vec<FileStat>> {
    let changes = repo.diff_tree_to_tree(Some(old_tree), Some(new_tree), None)?;
    let read = |id: Option<gix::ObjectId>| id.map(|id| diff::blob(repo, id)).unwrap_or_default();
    let mut files: Vec<FileStat> = changes
        .into_iter()
        .filter_map(|change| {
            let (kind, path, from, old, new, mode) = match change {
                ChangeDetached::Addition { location, id, entry_mode, .. } => {
                    (ChangeKind::Added, location, None, None, Some(id), entry_mode)
                }
                ChangeDetached::Deletion { location, id, entry_mode, .. } => {
                    (ChangeKind::Deleted, location, None, Some(id), None, entry_mode)
                }
                ChangeDetached::Modification { location, previous_id, id, entry_mode, .. } => {
                    (ChangeKind::Modified, location, None, Some(previous_id), Some(id), entry_mode)
                }
                ChangeDetached::Rewrite { source_location, source_id, location, id, copy, entry_mode, .. } => {
                    let kind = if copy { ChangeKind::Copied } else { ChangeKind::Renamed };
                    (kind, location, Some(source_location), Some(source_id), Some(id), entry_mode)
                }
            };
            // Folders show up as the files in them.
            (!mode.is_tree()).then(|| FileStat {
                kind,
                path: path.to_str_lossy().into_owned(),
                from: from.map(|f| f.to_str_lossy().into_owned()),
                size: if mode.is_commit() { None } else { diff::count(&read(old), &read(new)) },
                old: old.filter(|_| !mode.is_commit()),
                new: new.filter(|_| !mode.is_commit()),
            })
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
