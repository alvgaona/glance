use anyhow::Result;
use gix::bstr::ByteSlice;
use gix::status::index_worktree::iter::Summary;

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

#[derive(Debug)]
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

pub fn read(repo: &gix::Repository) -> Result<Status> {
    let mut st = Status::default();

    let head_ref = repo.head_name()?;
    st.branch = head_ref.as_ref().map(|n| n.shorten().to_string());
    st.head = repo.head_id().ok().map(|id| id.shorten_or_id().to_string());
    st.in_progress = repo.state().map(|s| format!("{s:?}").to_lowercase());
    st.stashes = count_stashes(repo);

    if let (Some(name), Ok(head_id)) = (&head_ref, repo.head_id()) {
        st.upstream = upstream(repo, name.as_ref(), head_id.detach()).ok().flatten();
    }

    let iter = repo
        .status(gix::progress::Discard)?
        .untracked_files(gix::status::UntrackedFiles::Collapsed)
        .into_iter(None)?;

    for item in iter {
        match item? {
            gix::status::Item::TreeIndex(change) => {
                use gix::diff::index::ChangeRef::*;
                let fc = match change {
                    Addition { location, .. } => file(ChangeKind::Added, &location, None),
                    Deletion { location, .. } => file(ChangeKind::Deleted, &location, None),
                    Modification { location, .. } => file(ChangeKind::Modified, &location, None),
                    Rewrite {
                        source_location,
                        location,
                        copy,
                        ..
                    } => {
                        let kind = if copy { ChangeKind::Copied } else { ChangeKind::Renamed };
                        file(kind, &location, Some(&source_location))
                    }
                };
                st.staged.push(fc);
            }
            gix::status::Item::IndexWorktree(item) => {
                let path = item.rela_path().to_str_lossy().into_owned();
                let kind = match item.summary() {
                    None => continue,
                    Some(Summary::Added) => {
                        st.untracked.push(path);
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
                st.unstaged.push(FileChange { kind, path, from: None });
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
    }
}

fn upstream(
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

    let count = |from: gix::ObjectId, hide: gix::ObjectId| -> Result<usize> {
        Ok(repo.rev_walk([from]).with_hidden([hide]).all()?.count())
    };
    Ok(Some(Upstream {
        name: name.shorten().to_string(),
        ahead: count(head, up)?,
        behind: count(up, head)?,
    }))
}

fn count_stashes(repo: &gix::Repository) -> usize {
    // The stash list is the reflog of refs/stash: one line per entry.
    std::fs::read(repo.common_dir().join("logs/refs/stash"))
        .map(|b| b.lines().count())
        .unwrap_or(0)
}
