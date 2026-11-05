//! Staging: the two halves of a file's change (HEAD → index, index → working tree) and
//! the writes that move changes between them.

use std::path::Path;

use anyhow::Result;
use gix::bstr::BStr;

use super::diff::{self, PatchLine};
use super::run::{args, git};

/// Which half of a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// HEAD → index: what the next commit takes.
    Staged,
    /// Index → working tree (untracked files included): what it leaves out.
    Unstaged,
}

/// Before and after of `path` on one side. `from` is the old path of a staged rename.
/// Missing on a side (new, deleted, untracked, unborn HEAD) reads as empty.
pub fn contents(repo: &gix::Repository, path: &str, from: Option<&str>, side: Side) -> Result<(Vec<u8>, Vec<u8>)> {
    let index = repo.index_or_empty()?;
    let in_index =
        |p: &str| index.entry_by_path(AsRef::<BStr>::as_ref(p)).map(|e| diff::blob(repo, e.id)).unwrap_or_default();
    Ok(match side {
        Side::Staged => {
            let old = match repo.head_tree() {
                Ok(tree) => match tree.lookup_entry_by_path(from.unwrap_or(path))? {
                    Some(e) => diff::blob(repo, e.object_id()),
                    None => Vec::new(),
                },
                // Unborn branch: nothing committed yet.
                Err(_) => Vec::new(),
            };
            (old, in_index(path))
        }
        Side::Unstaged => {
            let disk = repo.workdir().map(|d| on_disk(&d.join(path))).unwrap_or_default();
            (in_index(path), disk)
        }
    })
}

/// A file's bytes as git sees them: a symlink is its target, a missing file is empty.
fn on_disk(p: &Path) -> Vec<u8> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => {
            std::fs::read_link(p).map(|t| t.to_string_lossy().into_owned().into_bytes()).unwrap_or_default()
        }
        Ok(m) if m.is_file() => std::fs::read(p).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The diff of one half of a file, 3 lines of context. `None` for binary or huge files.
pub fn patch(repo: &gix::Repository, path: &str, from: Option<&str>, side: Side) -> Result<Option<Vec<PatchLine>>> {
    let (old, new) = contents(repo, path, from, side)?;
    Ok(diff::patch(&old, &new, 3))
}

fn with_paths(cmd: &[&str], paths: &[String]) -> Vec<String> {
    let mut a = args(cmd);
    a.push("--".into());
    a.extend(paths.iter().cloned());
    a
}

/// Stages whole files: changes, new files and deletions.
pub fn add(paths: &[String]) -> (bool, String) {
    git(&with_paths(&["add", "--all"], paths))
}

/// Takes whole files out of the index again; the working tree keeps them.
/// `unborn`: nothing is committed yet, so there's no HEAD to go back to.
pub fn unstage(paths: &[String], unborn: bool) -> (bool, String) {
    if unborn {
        git(&with_paths(&["rm", "--cached", "-r", "-q"], paths))
    } else {
        git(&with_paths(&["restore", "--staged"], paths))
    }
}

pub fn add_all() -> (bool, String) {
    git(&args(&["add", "--all"]))
}

pub fn unstage_all(unborn: bool) -> (bool, String) {
    if unborn { git(&args(&["rm", "--cached", "-r", "-q", "."])) } else { git(&args(&["reset", "-q"])) }
}

/// Throws away unstaged changes to tracked files: back to what's staged.
pub fn discard(paths: &[String]) -> (bool, String) {
    git(&with_paths(&["restore", "--worktree"], paths))
}

/// Deletes untracked files or folders (never ignored ones).
pub fn delete_untracked(paths: &[String]) -> (bool, String) {
    git(&with_paths(&["clean", "-f", "-d", "-q"], paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// A scratch repo, removed when dropped.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("gg-stage-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let s = Scratch(dir);
            s.git(&["init", "-q", "-b", "main"]);
            s
        }
        fn git(&self, a: &[&str]) {
            let ok = Command::new("git")
                .args(a)
                .current_dir(&self.0)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {a:?}");
        }
        /// Runs one of our writes inside the repo.
        fn run(&self, f: impl FnOnce() -> (bool, String)) {
            let here = std::env::current_dir().unwrap();
            let _guard = CWD.lock().unwrap();
            std::env::set_current_dir(&self.0).unwrap();
            let (ok, out) = f();
            std::env::set_current_dir(here).unwrap();
            assert!(ok, "{out}");
        }
        fn write(&self, path: &str, text: &str) {
            std::fs::write(self.0.join(path), text).unwrap();
        }
        fn repo(&self) -> gix::Repository {
            gix::open(&self.0).unwrap()
        }
        fn halves(&self, path: &str) -> (String, String, String) {
            let repo = self.repo();
            let s = |b: Vec<u8>| String::from_utf8(b).unwrap();
            let (head, index) = contents(&repo, path, None, Side::Staged).unwrap();
            let (index2, disk) = contents(&repo, path, None, Side::Unstaged).unwrap();
            assert_eq!(index, index2);
            (s(head), s(index), s(disk))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Tests changing the working directory take turns.
    static CWD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn reads_both_halves() {
        let s = Scratch::new("halves");
        s.write("a.txt", "one\n");
        s.git(&["add", "a.txt"]);
        s.git(&["commit", "-q", "-m", "one"]);
        s.write("a.txt", "two\n");
        s.git(&["add", "a.txt"]);
        s.write("a.txt", "three\n");
        assert_eq!(s.halves("a.txt"), ("one\n".into(), "two\n".into(), "three\n".into()));

        let p = patch(&s.repo(), "a.txt", None, Side::Unstaged).unwrap().unwrap();
        assert!(matches!(&p[1], PatchLine::Removed { text, .. } if text == "two"));
        assert!(matches!(&p[2], PatchLine::Added { text, .. } if text == "three"));

        // A file that's only on disk.
        s.write("new.txt", "hi\n");
        assert_eq!(s.halves("new.txt"), (String::new(), String::new(), "hi\n".into()));
    }

    #[test]
    fn stages_unstages_and_discards() {
        let s = Scratch::new("writes");
        s.write("a.txt", "one\n");
        // Unborn: unstaging can't go back to HEAD.
        s.run(|| add(&["a.txt".into()]));
        assert_eq!(s.halves("a.txt").1, "one\n");
        s.run(|| unstage(&["a.txt".into()], true));
        assert_eq!(s.halves("a.txt").1, "");
        assert_eq!(s.halves("a.txt").2, "one\n");

        s.run(add_all);
        s.git(&["commit", "-q", "-m", "one"]);
        s.write("a.txt", "two\n");
        s.run(|| add(&["a.txt".into()]));
        s.run(|| unstage(&["a.txt".into()], false));
        assert_eq!(s.halves("a.txt"), ("one\n".into(), "one\n".into(), "two\n".into()));

        s.run(|| discard(&["a.txt".into()]));
        assert_eq!(s.halves("a.txt").2, "one\n");

        // Deletions stage too.
        std::fs::remove_file(s.0.join("a.txt")).unwrap();
        s.run(|| add(&["a.txt".into()]));
        assert_eq!(s.halves("a.txt"), ("one\n".into(), String::new(), String::new()));
        s.run(|| unstage_all(false));
        assert_eq!(s.halves("a.txt").1, "one\n");

        // Untracked files and folders go; ignored ones stay.
        std::fs::create_dir(s.0.join("dir")).unwrap();
        s.write("dir/x.txt", "x\n");
        s.write("loose.txt", "x\n");
        s.write(".gitignore", "kept.log\n");
        s.write("kept.log", "x\n");
        s.run(|| delete_untracked(&["dir/".into(), "loose.txt".into()]));
        assert!(!s.0.join("dir").exists() && !s.0.join("loose.txt").exists());
        assert!(s.0.join("kept.log").exists());
    }
}
