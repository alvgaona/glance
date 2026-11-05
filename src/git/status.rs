use anyhow::Result;
use gix::bstr::ByteSlice;
use gix::status::index_worktree::iter::Summary;

use super::diff::{self, Size};

#[derive(Debug, Default)]
pub struct Status {
    /// Branch name, or `None` when HEAD is detached.
    pub branch: Option<String>,
    /// Short id of HEAD, `None` on an unborn branch.
    pub head: Option<String>,
    pub upstream: Option<Upstream>,
    pub stashes: usize,
    /// Rebase, merge, cherry-pick... if one is in progress.
    pub in_progress: Option<String>,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub untracked: Vec<String>,
    pub conflicted: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Upstream {
    pub name: String,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Debug)]
pub struct FileChange {
    pub kind: ChangeKind,
    pub path: String,
    /// Set for renames and copies.
    pub from: Option<String>,
    /// Lines added/removed; `None` when not computed.
    pub size: Option<Size>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChange,
}

/// With `sizes`, also counts added/removed lines per file (slower; `gg repos` skips it).
pub fn read(repo: &gix::Repository, sizes: bool) -> Result<Status> {
    let mut st = Status::default();
    let workdir = repo.workdir().map(|d| d.to_path_buf());
    let mut disk = if sizes { super::stage::Disk::new(repo) } else { None };

    let head_ref = repo.head_name()?;
    st.branch = head_ref.as_ref().map(|n| n.shorten().to_string());
    st.head = repo.head_id().ok().map(|id| id.shorten_or_id().to_string());
    st.in_progress = in_progress(repo);
    st.stashes = count_stashes(repo);

    if let (Some(name), Ok(head_id)) = (&head_ref, repo.head_id()) {
        st.upstream = upstream(repo, name.as_ref(), head_id.detach()).ok().flatten();
    }

    let iter =
        repo.status(gix::progress::Discard)?.untracked_files(gix::status::UntrackedFiles::Collapsed).into_iter(None)?;

    for item in iter {
        match item? {
            gix::status::Item::TreeIndex(change) => {
                use gix::diff::index::ChangeRef::*;
                // (kind, path, renamed-from, old blob, new blob)
                let (kind, path, from, old, new) = match change {
                    Addition { location, id, .. } => (ChangeKind::Added, location, None, None, Some(id.into_owned())),
                    Deletion { location, id, .. } => (ChangeKind::Deleted, location, None, Some(id.into_owned()), None),
                    Modification { location, previous_id, id, .. } => {
                        (ChangeKind::Modified, location, None, Some(previous_id.into_owned()), Some(id.into_owned()))
                    }
                    Rewrite { source_location, source_id, location, id, copy, .. } => {
                        let kind = if copy { ChangeKind::Copied } else { ChangeKind::Renamed };
                        (kind, location, Some(source_location), Some(source_id.into_owned()), Some(id.into_owned()))
                    }
                };
                let mut fc = file(kind, &path, from.as_deref());
                if sizes {
                    let read = |id: Option<gix::ObjectId>| id.map(|id| diff::blob(repo, id)).unwrap_or_default();
                    fc.size = diff::count(&read(old), &read(new));
                }
                st.staged.push(fc);
            }
            gix::status::Item::IndexWorktree(item) => {
                let path = item.rela_path().to_str_lossy().into_owned();
                // Index blob vs the file on disk.
                let size = match (&item, &workdir) {
                    (gix::status::index_worktree::Item::Modification { entry, .. }, Some(_)) if sizes => {
                        let on_disk = disk.as_mut().map(|d| d.read(&path)).unwrap_or_default();
                        diff::count(&diff::blob(repo, entry.id), &on_disk)
                    }
                    _ => None,
                };
                let kind = match item.summary() {
                    None => continue,
                    Some(Summary::Added) => {
                        // Collapsed folders get a trailing slash, like `git status`.
                        let is_dir = workdir.as_ref().is_some_and(|d| d.join(&path).is_dir());
                        st.untracked.push(if is_dir && !path.ends_with('/') { format!("{path}/") } else { path });
                        continue;
                    }
                    Some(Summary::Conflict) => {
                        st.conflicted.push(path);
                        continue;
                    }
                    Some(Summary::Removed) => ChangeKind::Deleted,
                    Some(Summary::Modified) => ChangeKind::Modified,
                    Some(Summary::TypeChange) => ChangeKind::TypeChange,
                    Some(Summary::Renamed) => ChangeKind::Renamed,
                    Some(Summary::Copied) => ChangeKind::Copied,
                    Some(Summary::IntentToAdd) => ChangeKind::Added,
                };
                st.unstaged.push(FileChange { kind, path, from: None, size });
            }
        }
    }

    st.staged.sort_by(|a, b| a.path.cmp(&b.path));
    st.unstaged.sort_by(|a, b| a.path.cmp(&b.path));
    st.untracked.sort();
    st.conflicted.sort();
    st.conflicted.dedup();
    Ok(st)
}

fn file(kind: ChangeKind, path: &gix::bstr::BStr, from: Option<&gix::bstr::BStr>) -> FileChange {
    FileChange {
        kind,
        path: path.to_str_lossy().into_owned(),
        from: from.map(|f| f.to_str_lossy().into_owned()),
        size: None,
    }
}

/// Upstream of `branch` with ahead/behind counts, if one is configured.
pub fn upstream(
    repo: &gix::Repository,
    branch: &gix::refs::FullNameRef,
    head: gix::ObjectId,
) -> Result<Option<Upstream>> {
    let Some(name) = repo.branch_remote_tracking_ref_name(branch, gix::remote::Direction::Fetch) else {
        return Ok(None);
    };
    let name = name?;
    let Ok(mut reference) = repo.find_reference(name.as_ref()) else {
        // Upstream is configured but was deleted on the remote.
        return Ok(Some(Upstream { name: format!("{} (gone)", name.shorten()), ahead: 0, behind: 0 }));
    };
    let up = reference.peel_to_id()?.detach();

    let (ahead, behind) = super::branches::ahead_behind(repo, head, up);
    Ok(Some(Upstream { name: name.shorten().to_string(), ahead, behind }))
}

/// "rebase 3/5", "merge", "cherry-pick"... for an operation that was started but not finished.
fn in_progress(repo: &gix::Repository) -> Option<String> {
    use gix::state::InProgress::*;
    let state = repo.state()?;
    let name = match state {
        Rebase | RebaseInteractive => "rebase",
        Merge => "merge",
        CherryPick | CherryPickSequence => "cherry-pick",
        Revert | RevertSequence => "revert",
        Bisect => "bisect",
        ApplyMailbox | ApplyMailboxRebase => "am",
    };
    if name != "rebase" {
        return Some(name.into());
    }
    // Files under rebase-merge/ for -i and merge rebases, rebase-apply/ otherwise.
    let text = |f: &str| {
        ["rebase-merge", "rebase-apply"]
            .iter()
            .find_map(|d| std::fs::read_to_string(repo.path().join(d).join(f)).ok())
            .map(|s| s.trim().to_string())
    };
    let num = |a: &str, b: &str| text(a).or_else(|| text(b)).and_then(|s| s.parse::<u32>().ok());
    let mut label = "rebasing".to_string();
    if let Some(branch) = text("head-name") {
        label += &format!(" {}", branch.trim_start_matches("refs/heads/"));
    }
    if let (Some(n), Some(total)) = (num("msgnum", "next"), num("end", "last")) {
        label += &format!(" {n}/{total}");
    }
    Some(label)
}

pub fn count_stashes(repo: &gix::Repository) -> usize {
    // The stash list is the reflog of refs/stash: one line per entry.
    std::fs::read(repo.common_dir().join("logs/refs/stash")).map(|b| b.lines().count()).unwrap_or(0)
}
