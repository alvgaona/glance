//! Files that differ between two trees (a commit and its parent, a stash and its base).

use anyhow::Result;
use gix::bstr::ByteSlice;
use gix::object::tree::diff::ChangeDetached;

use super::diff::{self, Size};
use super::status::ChangeKind;

#[derive(Debug, Clone)]
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

/// One changed file as `git diff` text, for delta.
pub fn file_text(repo: &gix::Repository, f: &FileStat) -> Vec<u8> {
    let read = |id: Option<gix::ObjectId>| id.map(|id| diff::blob(repo, id)).unwrap_or_default();
    let from = f.from.as_deref().unwrap_or(&f.path);
    let a = if f.old.is_some() { format!("a/{from}") } else { "/dev/null".into() };
    let b = if f.new.is_some() { format!("b/{}", f.path) } else { "/dev/null".into() };
    let mut out = format!("diff --git a/{from} b/{}\n", f.path).into_bytes();
    match diff::hunks(&read(f.old), &read(f.new), 3) {
        // shortcut: huge text files say "binary" too, the same as the built-in view's "binary or very large".
        None => out.extend(format!("Binary files {a} and {b} differ\n").into_bytes()),
        Some(hunks) => {
            out.extend(format!("--- {a}\n+++ {b}\n").into_bytes());
            hunks.iter().for_each(|h| out.extend(h.text()));
        }
    }
    out
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
