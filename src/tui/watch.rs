//! Notices changes made outside glance (another terminal, an editor, a hook) so the
//! screen on show can reload. A thread stamps the repository every second; the loop in
//! `app` compares stamps.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const EVERY: Duration = Duration::from_secs(1);

/// Starts stamping `repo`: each stamp comes with when its reading began. `edits`: edits
/// in the working tree count too, not only the index and refs. Stops when the receiver
/// is dropped.
pub(crate) fn start(repo: &gix::Repository, edits: bool) -> Receiver<(Instant, u64)> {
    let (tx, rx) = mpsc::channel();
    let git = repo.git_dir().to_path_buf();
    let common = repo.common_dir().to_path_buf();
    let work = if edits { repo.workdir().map(Path::to_path_buf) } else { None };
    std::thread::spawn(move || {
        while tx.send((Instant::now(), stamp(&git, &common, work.as_deref()))).is_ok() {
            std::thread::sleep(EVERY);
        }
    });
    rx
}

/// What the repository looks like, in one number: sizes and times of HEAD, the index,
/// the refs, the config and the other worktrees' files; with `work`, of each changed
/// or untracked file too.
fn stamp(git: &Path, common: &Path, work: Option<&Path>) -> u64 {
    let mut h = DefaultHasher::new();
    for f in [git.join("HEAD"), git.join("index"), common.join("packed-refs"), common.join("config")] {
        file(&f, &mut h);
    }
    for d in [common.join("refs"), common.join("worktrees")] {
        walk(&d, &mut h);
    }
    if let Some(w) = work {
        // Reads the index without writing it, unlike `git status`, so it can't wake itself.
        let args = crate::git::run::args(&["ls-files", "--modified", "--others", "--exclude-standard", "-z"]);
        if let Ok(o) = crate::process::run_with_input("git", &args, Some(w), b"", &[]) {
            o.stdout.hash(&mut h);
            for p in o.stdout.split('\0').filter(|p| !p.is_empty()) {
                file(&w.join(p), &mut h);
            }
        }
    }
    h.finish()
}

fn file(p: &Path, h: &mut impl Hasher) {
    p.hash(h);
    if let Ok(m) = std::fs::metadata(p) {
        m.len().hash(h);
        m.modified().ok().hash(h);
    }
}

fn walk(dir: &Path, h: &mut impl Hasher) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() { walk(&p, h) } else { file(&p, h) }
    }
}

#[cfg(test)]
mod tests {
    use super::stamp;
    use crate::git::run::Scratch;

    #[test]
    fn edits_commits_and_branches_change_the_stamp() {
        let s = Scratch::new("watch-stamp");
        s.commit("a.txt", "one\n");
        let repo = s.repo();
        let (git, common, work) = (repo.git_dir(), repo.common_dir(), repo.workdir());
        let now = || stamp(git, common, work);
        let refs = || stamp(git, common, None);

        let first = now();
        assert_eq!(first, now(), "nothing changed");

        s.write("a.txt", "one two\n");
        let edited = now();
        assert_ne!(first, edited, "an edit");
        s.write("a.txt", "one two three\n");
        assert_ne!(edited, now(), "a second edit to a file already changed");

        let before = refs();
        s.write("b.txt", "new\n");
        assert_eq!(before, refs(), "edits don't count without the working tree");
        s.git(&["branch", "other"]);
        assert_ne!(before, refs(), "a new branch");
        let before = refs();
        s.git(&["add", "-A"]);
        s.git(&["commit", "-q", "-m", "more"]);
        assert_ne!(before, refs(), "a commit");
    }
}
