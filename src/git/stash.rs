//! The stash list (the reflog of `refs/stash`) and what each stash holds.
//!
//! A stash is a commit whose tree is the working tree at stash time. Its parents are
//! the commit it was made on, the index, and (with `-u`) a commit holding untracked files.

use anyhow::Result;
use gix::ObjectId;
use gix::bstr::ByteSlice;

use super::run::{self, Ran};
use super::tree::{self, FileStat};

#[derive(Debug, Clone)]
pub struct Stash {
    /// `n` in `stash@{n}`; 0 is the newest.
    pub index: usize,
    pub oid: ObjectId,
    pub id: String,
    /// What the user wrote, or git's "WIP on main: abc123 subject".
    pub message: String,
    /// Branch it was made on, from the message ("WIP on main:", "On main:").
    pub branch: Option<String>,
    pub time: i64,
    /// The commit it was made on.
    pub base: ObjectId,
    pub base_id: String,
    pub base_subject: String,
    /// Commit with untracked files, when stashed with `-u`.
    pub untracked: Option<ObjectId>,
}

impl Stash {
    pub fn name(&self) -> String {
        format!("stash@{{{}}}", self.index)
    }

    /// The message without git's "WIP on main: abc123 " / "On main: " prefix.
    pub fn title(&self) -> &str {
        let m = self.message.as_str();
        if let Some(rest) = m.strip_prefix("WIP on ") {
            // "WIP on main: abc123 subject" -> "subject"
            let after = rest.split_once(": ").map_or(rest, |(_, r)| r);
            return after.split_once(' ').map_or(after, |(_, s)| s);
        }
        if let Some(rest) = m.strip_prefix("On ") {
            return rest.split_once(": ").map_or(rest, |(_, r)| r);
        }
        m
    }

    /// Made by `git stash` without `-m`: the title is just the last commit's subject.
    pub fn is_wip(&self) -> bool {
        self.message.starts_with("WIP on ")
    }
}

/// Newest first, like `git stash list`.
pub fn list(repo: &gix::Repository) -> Result<Vec<Stash>> {
    let Ok(r) = repo.find_reference("refs/stash") else {
        return Ok(Vec::new());
    };
    let mut platform = r.log_iter();
    let Some(lines) = platform.rev()? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (index, line) in lines.enumerate() {
        let line = line?;
        let oid = line.new_oid;
        let Ok(commit) = repo.find_commit(oid) else { continue };
        let parents: Vec<ObjectId> = commit.parent_ids().map(|p| p.detach()).collect();
        let Some(&base) = parents.first() else { continue };
        let base_commit = repo.find_commit(base)?;
        let message = line.message.to_str_lossy().into_owned();
        let branch = message
            .strip_prefix("WIP on ")
            .or_else(|| message.strip_prefix("On "))
            .and_then(|r| r.split_once(':'))
            .map(|(b, _)| b.to_string())
            .filter(|b| b != "(no branch)");
        out.push(Stash {
            index,
            oid,
            id: commit.id().shorten_or_id().to_string(),
            branch,
            time: line.signature.time.seconds,
            base,
            base_id: base_commit.id().shorten_or_id().to_string(),
            base_subject: base_commit.message()?.summary().to_str_lossy().into_owned(),
            // `-u` adds this commit even when nothing was untracked; an empty one doesn't count.
            untracked: parents.get(2).copied().filter(|&u| {
                repo.find_commit(u)
                    .ok()
                    .and_then(|c| c.tree().ok())
                    .is_some_and(|t| t.decode().is_ok_and(|d| !d.entries.is_empty()))
            }),
            message,
        });
    }
    Ok(out)
}

/// What the stash changes on top of its base, and the untracked files it holds.
pub fn files(repo: &gix::Repository, s: &Stash) -> Result<(Vec<FileStat>, Vec<FileStat>)> {
    let tree = repo.find_commit(s.oid)?.tree()?;
    let base = repo.find_commit(s.base)?.tree()?;
    let tracked = tree::changes(repo, &base, &tree)?;
    let untracked = match s.untracked {
        Some(u) => tree::changes(repo, &repo.empty_tree(), &repo.find_commit(u)?.tree()?)?,
        None => Vec::new(),
    };
    Ok((tracked, untracked))
}

// ---------------------------------------------------------------- writes

/// Which changes `git stash push` takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Staged and unstaged changes (and untracked files, if asked).
    Everything,
    /// Only what's staged: `--staged`. git ignores file lists here and refuses files
    /// that also have unstaged changes.
    Staged,
    /// Everything but the staged part, which stays in place: `--keep-index`. git ignores
    /// file lists here too, and still records the whole index.
    Unstaged,
}

/// What to stash. `paths: None` means everything the mode covers.
#[derive(Clone, Debug)]
pub struct Push {
    pub mode: Mode,
    pub untracked: bool,
    pub paths: Option<Vec<String>>,
}

impl Push {
    pub fn args(&self, message: &str) -> Vec<String> {
        let mut a = run::args(&["stash", "push"]);
        match self.mode {
            Mode::Everything => {}
            Mode::Staged => a.push("--staged".into()),
            Mode::Unstaged => a.push("--keep-index".into()),
        }
        // A staged-only stash can't hold untracked files.
        if self.untracked && self.mode != Mode::Staged {
            a.push("--include-untracked".into());
        }
        if !message.is_empty() {
            a.extend(["-m".into(), message.into()]);
        }
        if let Some(paths) = &self.paths {
            a.push("--".into());
            a.extend(paths.iter().map(|p| run::top(p)));
        }
        a
    }

    pub fn run(&self, message: &str) -> Ran {
        run::git(&self.args(message))
    }
}

/// `git stash apply` (or `pop`, which also drops it when it applies cleanly).
pub fn apply(s: &Stash, pop: bool) -> Ran {
    run::git(&run::args(&["stash", if pop { "pop" } else { "apply" }, &s.name()]))
}

pub fn drop(s: &Stash) -> Ran {
    run::git(&run::args(&["stash", "drop", &s.name()]))
}

/// `git stash branch`: a new branch at the stash's base, with the stash applied and dropped.
pub fn branch(s: &Stash, name: &str) -> Ran {
    run::git(&run::args(&["stash", "branch", name, &s.name()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(mode: Mode, untracked: bool, paths: Option<Vec<&str>>) -> Push {
        Push { mode, untracked, paths: paths.map(|p| p.into_iter().map(String::from).collect()) }
    }

    #[test]
    fn push_args() {
        assert_eq!(push(Mode::Everything, true, None).args(""), ["stash", "push", "--include-untracked"]);
        assert_eq!(
            push(Mode::Everything, false, Some(vec!["a.rs"])).args("wip"),
            ["stash", "push", "-m", "wip", "--", ":(top,literal)a.rs"]
        );
        assert_eq!(push(Mode::Staged, true, None).args(""), ["stash", "push", "--staged"]);
        assert_eq!(push(Mode::Unstaged, true, None).args(""), ["stash", "push", "--keep-index", "--include-untracked"]);
    }

    fn stash(message: &str) -> Stash {
        Stash {
            index: 0,
            oid: ObjectId::null(gix::hash::Kind::Sha1),
            id: String::new(),
            message: message.into(),
            branch: None,
            time: 0,
            base: ObjectId::null(gix::hash::Kind::Sha1),
            base_id: String::new(),
            base_subject: String::new(),
            untracked: None,
        }
    }

    #[test]
    fn titles() {
        assert_eq!(stash("WIP on main: e6f5685 Local work").title(), "Local work");
        assert_eq!(stash("On feat/x: try the new parser").title(), "try the new parser");
        assert_eq!(stash("On main: a: b").title(), "a: b");
        assert_eq!(stash("something else").title(), "something else");
        assert!(stash("WIP on main: e6f5685 Local work").is_wip());
    }
}
