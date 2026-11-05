//! One commit in full: message, author, refs and the files it changed. And making one.

use std::path::Path;

use anyhow::{Context, Result};
use gix::bstr::ByteSlice;

use super::log::{RefLabel, ref_labels};
use super::run::{args, git_on_terminal, git_with_input};
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

/// Commits what's staged (or amends HEAD) with `message`, output collected; runs in the
/// repository root so hooks see the usual folder.
pub fn create(repo: &gix::Repository, message: &str, amend: bool) -> (bool, String) {
    let mut a = args(&["commit", "-F", "-"]);
    if amend {
        a.push("--amend".into());
    }
    git_with_input(repo.workdir(), &a, message.as_bytes())
}

/// The same on the terminal: signing may ask for a passphrase, and with `edit` git
/// opens the editor on the message first. The message waits in a file meanwhile.
pub fn create_on_terminal(repo: &gix::Repository, message: &str, amend: bool, edit: bool) -> (bool, String) {
    let file = repo.git_dir().join("GG_COMMIT_MSG");
    if let Err(e) = std::fs::write(&file, message) {
        return (false, format!("couldn't write {}: {e}", file.display()));
    }
    let r = git_on_terminal(repo.workdir(), &create_args(&file, amend, edit));
    let _ = std::fs::remove_file(&file);
    r
}

fn create_args(file: &Path, amend: bool, edit: bool) -> Vec<String> {
    let mut a = args(&["commit", "-F"]);
    a.push(file.display().to_string());
    if amend {
        a.push("--amend".into());
    }
    if edit {
        a.push("--edit".into());
    }
    a
}

/// Are commits signed? Signing may prompt for a passphrase.
pub fn signs(repo: &gix::Repository) -> bool {
    repo.config_snapshot().boolean("commit.gpgsign").unwrap_or(false)
}

/// HEAD's message, for amending.
pub fn last_message(repo: &gix::Repository) -> Option<String> {
    let commit = repo.head_commit().ok()?;
    let m = commit.message_raw().ok()?;
    Some(m.to_str_lossy().trim_end().to_string())
}

/// The message git prepared for a merge, cherry-pick or revert in progress, comments
/// taken out.
pub fn prepared_message(repo: &gix::Repository) -> Option<String> {
    let text = std::fs::read_to_string(repo.git_dir().join("MERGE_MSG")).ok()?;
    let kept: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    let text = kept.join("\n").trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_and_amends() {
        let dir = std::env::temp_dir().join(format!("gg-commit-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |a: &[&str]| {
            let ok = std::process::Command::new("git").args(a).current_dir(&dir).status().unwrap().success();
            assert!(ok, "git {a:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "t"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        git(&["add", "a.txt"]);

        let repo = gix::open(&dir).unwrap();
        assert!(!signs(&repo));
        let (ok, out) = create(&repo, "feat: first\n\nWhy it matters.", false);
        assert!(ok, "{out}");
        let repo = gix::open(&dir).unwrap();
        assert_eq!(last_message(&repo).as_deref(), Some("feat: first\n\nWhy it matters."));

        // Amending with nothing staged changes only the message.
        let (ok, out) = create(&repo, "feat: better first", true);
        assert!(ok, "{out}");
        let repo = gix::open(&dir).unwrap();
        assert_eq!(last_message(&repo).as_deref(), Some("feat: better first"));
        assert_eq!(repo.head_commit().unwrap().parent_ids().count(), 0);

        // Nothing staged: git refuses, and says so.
        let (ok, out) = create(&repo, "empty", false);
        assert!(!ok);
        assert!(out.contains("nothing"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
