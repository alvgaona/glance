//! Staging: the two halves of a file's change (HEAD → index, index → working tree) and
//! the writes that move changes between them.

use std::path::Path;

use anyhow::Result;
use gix::bstr::BStr;

pub use super::diff::Hunk;
use super::diff::{self, Op};
use super::run::{Ran, args, git_in, git_with_input, top};

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
            let disk = Disk::new(repo).map(|mut d| d.read(path)).unwrap_or_default();
            (in_index(path), disk)
        }
    })
}

/// Reads working tree files as git would store them.
pub struct Disk<'r> {
    dir: std::path::PathBuf,
    filters: gix::filter::Pipeline<'r>,
    index: gix::worktree::IndexPersistedOrInMemory,
}

impl<'r> Disk<'r> {
    /// `None` for a bare repository.
    pub fn new(repo: &'r gix::Repository) -> Option<Self> {
        let dir = repo.workdir()?.to_path_buf();
        let (filters, index) = repo.filter_pipeline(None).ok()?;
        Some(Disk { dir, filters, index })
    }

    /// `path`'s bytes as git sees them: a symlink is its target, a missing file is
    /// empty, and a file goes through its filters (`eol=crlf`, LFS…), as `git add`
    /// would put it in the index.
    pub fn read(&mut self, path: &str) -> Vec<u8> {
        use std::io::Read;
        let p = self.dir.join(path);
        let raw = match std::fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => {
                return std::fs::read_link(&p)
                    .map(|t| t.to_string_lossy().into_owned().into_bytes())
                    .unwrap_or_default();
            }
            Ok(m) if m.is_file() => std::fs::read(&p).unwrap_or_default(),
            _ => return Vec::new(),
        };
        let mut out = Vec::new();
        let converted = self
            .filters
            .convert_to_git(raw.as_slice(), Path::new(path), &self.index)
            .is_ok_and(|mut c| c.read_to_end(&mut out).is_ok());
        if converted { out } else { raw }
    }
}

/// The diff of one half of a file, 3 lines of context, hunk by hunk and byte for byte
/// (so parts of it can be staged). `None` for binary or huge files.
pub fn hunks(repo: &gix::Repository, path: &str, from: Option<&str>, side: Side) -> Result<Option<Vec<Hunk>>> {
    let (old, new) = contents(repo, path, from, side)?;
    Ok(diff::hunks(&old, &new, 3))
}

/// What to do with one hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkAction {
    /// An unstaged hunk into the index.
    Stage,
    /// A staged hunk out of the index again.
    Unstage,
    /// An unstaged hunk thrown away from the working tree.
    Discard,
}

/// The patch `git apply` needs for one hunk of `path`.
pub fn hunk_patch(path: &str, h: &Hunk) -> Vec<u8> {
    let (a, b) = (quoted(&format!("a/{path}")), quoted(&format!("b/{path}")));
    let mut out = format!("diff --git {a} {b}\n--- {a}\n+++ {b}\n").into_bytes();
    out.extend(h.text());
    out
}

/// `h` cut down to the changed lines at `picked` (indices into `h.lines`), for `action`.
/// The rest must stay as the file the patch lands on has it: staging goes forward onto
/// the index, so an unpicked removal stays (becomes context) and an unpicked addition
/// goes; unstaging and discarding go backwards, so it's the other way round.
///
/// In a run of changes, the n-th removal pairs with the n-th addition (its
/// replacement), and a line kept as context goes after that pair's picked lines:
/// staging "1 → one" out of "1 2 → one two" gives "one 2", not "2 one".
pub fn pick_lines(h: &Hunk, picked: &[usize], action: HunkAction) -> Hunk {
    let forward = action == HunkAction::Stage;
    let mut out = Hunk { lines: Vec::new(), ..h.clone() };
    let mut i = 0;
    while i < h.lines.len() {
        if h.lines[i].op == Op::Context {
            out.lines.push(h.lines[i].clone());
            i += 1;
            continue;
        }
        // One run: its removals, then its additions.
        let end = (i..h.lines.len()).find(|&j| h.lines[j].op == Op::Context).unwrap_or(h.lines.len());
        let removed: Vec<usize> = (i..end).filter(|&j| h.lines[j].op == Op::Removed).collect();
        let added: Vec<usize> = (i..end).filter(|&j| h.lines[j].op == Op::Added).collect();
        for n in 0..removed.len().max(added.len()) {
            let mut kept_as_context = None;
            for j in [removed.get(n), added.get(n)].into_iter().flatten().copied() {
                let l = &h.lines[j];
                if picked.contains(&j) {
                    out.lines.push(l.clone());
                } else if (l.op == Op::Removed) == forward {
                    // Still in the file the patch lands on.
                    let mut l = l.clone();
                    l.op = Op::Context;
                    kept_as_context = Some(l);
                }
            }
            out.lines.extend(kept_as_context);
        }
        i = end;
    }
    out
}

/// Does the hunk still change anything?
pub fn changes_anything(h: &Hunk) -> bool {
    h.lines.iter().any(|l| l.op != Op::Context)
}

/// A path for a patch header, quoted the way git quotes it when it has a tab, a
/// newline, `"` or `\`: unquoted, `git apply` would cut the name at the tab.
fn quoted(path: &str) -> String {
    if !path.chars().any(|c| c == '"' || c == '\\' || c.is_control()) {
        return path.to_string();
    }
    let inner: String = path
        .chars()
        .map(|c| match c {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            '\t' => "\\t".to_string(),
            '\n' => "\\n".to_string(),
            c if c.is_control() => format!("\\{:03o}", c as u32),
            c => c.to_string(),
        })
        .collect();
    format!("\"{inner}\"")
}

/// Can `git apply` take this hunk as it is? A line without a newline must be the last
/// line on its side, or git glues the next line onto it.
fn ends_cleanly(h: &Hunk) -> bool {
    [Op::Removed, Op::Added].into_iter().all(|side| {
        let lines: Vec<_> = h.lines.iter().filter(|l| l.op == Op::Context || l.op == side).collect();
        lines.iter().rev().skip(1).all(|l| l.bytes.ends_with(b"\n"))
    })
}

/// Applies one hunk of `path` (taken from the side the action works on).
pub fn apply_hunk(repo: &gix::Repository, path: &str, h: &Hunk, action: HunkAction) -> Ran {
    if !ends_cleanly(h) {
        return Err("Those lines touch the end of a line with no newline: take the whole hunk".into());
    }
    let how: &[&str] = match action {
        HunkAction::Stage => &["--cached"],
        HunkAction::Unstage => &["--cached", "--reverse"],
        HunkAction::Discard => &["--reverse"],
    };
    let mut a = args(&["apply", "--recount", "--whitespace=nowarn"]);
    a.extend(args(how));
    a.push("-".into());
    git_with_input(repo.workdir(), &a, &hunk_patch(path, h))
}

// The writes run in the repository's root folder: paths come from status, so they're
// relative to it, wherever gg was started.

fn run(repo: &gix::Repository, cmd: &[&str], paths: Option<&[String]>) -> Ran {
    let mut a = args(cmd);
    if let Some(paths) = paths {
        a.push("--".into());
        a.extend(paths.iter().map(|p| top(p)));
    }
    git_in(repo.workdir(), &a)
}

/// No commit yet: there's no HEAD to unstage back to.
fn unborn(repo: &gix::Repository) -> bool {
    repo.head_id().is_err()
}

/// `gg add <paths>`: plain `git add`, paths as typed (from the current folder).
pub fn add_typed(paths: &[String]) -> Ran {
    let mut a = args(&["add", "--"]);
    a.extend(paths.iter().cloned());
    git_in(None, &a)
}

/// Stages whole files: changes, new files and deletions.
pub fn add(repo: &gix::Repository, paths: &[String]) -> Ran {
    run(repo, &["add", "--all"], Some(paths))
}

/// Takes whole files out of the index again; the working tree keeps them.
pub fn unstage(repo: &gix::Repository, paths: &[String]) -> Ran {
    if unborn(repo) {
        run(repo, &["rm", "--cached", "-r", "-q"], Some(paths))
    } else {
        run(repo, &["restore", "--staged"], Some(paths))
    }
}

pub fn add_all(repo: &gix::Repository) -> Ran {
    run(repo, &["add", "--all"], None)
}

pub fn unstage_all(repo: &gix::Repository) -> Ran {
    if unborn(repo) { run(repo, &["rm", "--cached", "-r", "-q", "."], None) } else { run(repo, &["reset", "-q"], None) }
}

/// Throws away unstaged changes to tracked files: back to what's staged.
pub fn discard(repo: &gix::Repository, paths: &[String]) -> Ran {
    run(repo, &["restore", "--worktree"], Some(paths))
}

/// Deletes untracked files or folders (never ignored ones).
pub fn delete_untracked(repo: &gix::Repository, paths: &[String]) -> Ran {
    run(repo, &["clean", "-f", "-d", "-q"], Some(paths))
}

/// Which sides of a conflicted file have it: the merge base, HEAD ("ours") and the
/// incoming side ("theirs"). One missing means that side deleted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conflict {
    pub base: bool,
    pub ours: bool,
    pub theirs: bool,
}

impl Conflict {
    /// Both sides have it, so the file holds conflict markers to fix.
    pub fn in_content(&self) -> bool {
        self.ours && self.theirs
    }
}

/// Every conflicted path in the index, and its kind.
pub fn conflicts(repo: &gix::Repository) -> Result<std::collections::HashMap<String, Conflict>> {
    use gix::index::entry::Stage;
    let index = repo.index_or_empty()?;
    let mut out = std::collections::HashMap::new();
    for e in index.entries() {
        let stage = e.stage();
        if stage == Stage::Unconflicted {
            continue;
        }
        let path = e.path(&index).to_string();
        let c = out.entry(path).or_insert(Conflict { base: false, ours: false, theirs: false });
        match stage {
            Stage::Base => c.base = true,
            Stage::Ours => c.ours = true,
            Stage::Theirs => c.theirs = true,
            Stage::Unconflicted => {}
        }
    }
    Ok(out)
}

/// Deletes files from the index and the working tree; for a conflict, that's taking
/// the side that deleted it.
pub fn remove(repo: &gix::Repository, paths: &[String]) -> Ran {
    run(repo, &["rm", "-q", "--ignore-unmatch"], Some(paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::git::run::Scratch;

    impl Scratch {
        /// Runs one of our writes on the repo, opened afresh.
        fn run(&self, f: impl FnOnce(&gix::Repository) -> Ran) {
            if let Err(e) = f(&self.repo()) {
                panic!("{e}");
            }
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

        let h = hunks(&s.repo(), "a.txt", None, Side::Unstaged).unwrap().unwrap();
        let lines: Vec<(Op, &[u8])> = h[0].lines.iter().map(|l| (l.op, l.bytes.as_slice())).collect();
        assert_eq!(lines, [(Op::Removed, b"two\n".as_slice()), (Op::Added, b"three\n".as_slice())]);

        // A file that's only on disk.
        s.write("new.txt", "hi\n");
        assert_eq!(s.halves("new.txt"), (String::new(), String::new(), "hi\n".into()));
    }

    #[test]
    fn stages_unstages_and_discards() {
        let s = Scratch::new("writes");
        s.write("a.txt", "one\n");
        let a = || vec!["a.txt".to_string()];
        // Unborn: unstaging can't go back to HEAD.
        s.run(|r| add(r, &a()));
        assert_eq!(s.halves("a.txt").1, "one\n");
        s.run(|r| unstage(r, &a()));
        assert_eq!(s.halves("a.txt").1, "");
        assert_eq!(s.halves("a.txt").2, "one\n");
        s.run(add_all);
        s.run(unstage_all);
        assert_eq!(s.halves("a.txt").1, "");

        s.run(add_all);
        s.git(&["commit", "-q", "-m", "one"]);
        s.write("a.txt", "two\n");
        s.run(|r| add(r, &a()));
        s.run(|r| unstage(r, &a()));
        assert_eq!(s.halves("a.txt"), ("one\n".into(), "one\n".into(), "two\n".into()));

        s.run(|r| discard(r, &a()));
        assert_eq!(s.halves("a.txt").2, "one\n");

        // Deletions stage too.
        std::fs::remove_file(s.0.join("a.txt")).unwrap();
        s.run(|r| add(r, &a()));
        assert_eq!(s.halves("a.txt"), ("one\n".into(), String::new(), String::new()));
        s.run(unstage_all);
        assert_eq!(s.halves("a.txt").1, "one\n");

        // Untracked files and folders go; ignored ones stay.
        std::fs::create_dir(s.0.join("dir")).unwrap();
        s.write("dir/x.txt", "x\n");
        s.write("loose.txt", "x\n");
        s.write(".gitignore", "kept.log\n");
        s.write("kept.log", "x\n");
        s.run(|r| delete_untracked(r, &["dir/".into(), "loose.txt".into()]));
        assert!(!s.0.join("dir").exists() && !s.0.join("loose.txt").exists());
        assert!(s.0.join("kept.log").exists());
    }

    fn hunk(s: &Scratch, path: &str, side: Side, i: usize) -> Hunk {
        hunks(&s.repo(), path, None, side).unwrap().unwrap().swap_remove(i)
    }

    #[test]
    fn stages_unstages_and_discards_single_hunks() {
        let s = Scratch::new("hunks");
        // Lines 1 to 20, with line 2 and/or line 18 spelled out: two hunks far apart.
        let text = |two: bool, eighteen: bool| -> String {
            (1..=20)
                .map(|i| match i {
                    2 if two => "two\n".to_string(),
                    18 if eighteen => "eighteen\n".to_string(),
                    i => format!("{i}\n"),
                })
                .collect()
        };
        let (base, second, both) = (text(false, false), text(false, true), text(true, true));
        s.write("f.txt", &base);
        s.git(&["add", "f.txt"]);
        s.git(&["commit", "-q", "-m", "base"]);
        s.write("f.txt", &both);
        assert_eq!(hunks(&s.repo(), "f.txt", None, Side::Unstaged).unwrap().unwrap().len(), 2);

        // Only the second change goes in; the file on disk doesn't move.
        let h = hunk(&s, "f.txt", Side::Unstaged, 1);
        s.run(|r| apply_hunk(r, "f.txt", &h, HunkAction::Stage));
        assert_eq!(s.halves("f.txt"), (base.clone(), second.clone(), both.clone()));

        // And out again.
        let h = hunk(&s, "f.txt", Side::Staged, 0);
        s.run(|r| apply_hunk(r, "f.txt", &h, HunkAction::Unstage));
        assert_eq!(s.halves("f.txt"), (base.clone(), base.clone(), both.clone()));

        // Throw the first change away; the second stays on disk.
        let h = hunk(&s, "f.txt", Side::Unstaged, 0);
        s.run(|r| apply_hunk(r, "f.txt", &h, HunkAction::Discard));
        assert_eq!(s.halves("f.txt").2, second);
    }

    /// Indices of the lines in `h` whose text is one of `texts`.
    fn lines_of(h: &Hunk, texts: &[&str]) -> Vec<usize> {
        (0..h.lines.len()).filter(|&i| texts.iter().any(|t| h.lines[i].bytes == format!("{t}\n").as_bytes())).collect()
    }

    #[test]
    fn stages_unstages_and_discards_picked_lines() {
        let s = Scratch::new("lines");
        s.write("f.txt", "1\n2\n3\n");
        s.git(&["add", "f.txt"]);
        s.git(&["commit", "-q", "-m", "base"]);
        // One hunk: -1 -2 +one +two, then 3.
        s.write("f.txt", "one\ntwo\n3\n");

        // Stage only "1 → one".
        let h = hunk(&s, "f.txt", Side::Unstaged, 0);
        let part = pick_lines(&h, &lines_of(&h, &["1", "one"]), HunkAction::Stage);
        s.run(|r| apply_hunk(r, "f.txt", &part, HunkAction::Stage));
        assert_eq!(s.halves("f.txt"), ("1\n2\n3\n".into(), "one\n2\n3\n".into(), "one\ntwo\n3\n".into()));

        // A single added line on its own stages too (the removal stays put).
        let h = hunk(&s, "f.txt", Side::Unstaged, 0);
        let part = pick_lines(&h, &lines_of(&h, &["two"]), HunkAction::Stage);
        s.run(|r| apply_hunk(r, "f.txt", &part, HunkAction::Stage));
        assert_eq!(s.halves("f.txt").1, "one\ntwo\n2\n3\n");

        // Unstage just the "two" again.
        let h = hunk(&s, "f.txt", Side::Staged, 0);
        let part = pick_lines(&h, &lines_of(&h, &["two"]), HunkAction::Unstage);
        s.run(|r| apply_hunk(r, "f.txt", &part, HunkAction::Unstage));
        assert_eq!(s.halves("f.txt").1, "one\n2\n3\n");

        // Discard only the added "two" from the working tree, keeping the removal of 2.
        let h = hunk(&s, "f.txt", Side::Unstaged, 0);
        let part = pick_lines(&h, &lines_of(&h, &["two"]), HunkAction::Discard);
        assert!(changes_anything(&part));
        s.run(|r| apply_hunk(r, "f.txt", &part, HunkAction::Discard));
        assert_eq!(s.halves("f.txt").2, "one\n3\n");

        // Nothing picked: nothing to do.
        assert!(!changes_anything(&pick_lines(&h, &[], HunkAction::Stage)));
    }

    #[test]
    fn a_last_line_without_newline_is_never_glued_to_the_next() {
        let s = Scratch::new("nonewline");
        s.commit("f.txt", "a\nb");
        s.write("f.txt", "a\nb\nc\n");
        // Only "c": "b" would stay without its newline, with "c" after it.
        let h = hunk(&s, "f.txt", Side::Unstaged, 0);
        let part = pick_lines(&h, &lines_of(&h, &["c"]), HunkAction::Stage);
        assert!(apply_hunk(&s.repo(), "f.txt", &part, HunkAction::Stage).is_err());
        assert_eq!(s.halves("f.txt").1, "a\nb");
        // The whole hunk is fine.
        s.run(|r| apply_hunk(r, "f.txt", &h, HunkAction::Stage));
        assert_eq!(s.halves("f.txt").1, "a\nb\nc\n");
    }

    #[test]
    fn a_tab_in_the_name_is_quoted_in_the_patch() {
        let s = Scratch::new("tabname");
        s.commit("tab\tx.txt", "1\n");
        s.write("tab\tx.txt", "2\n");
        let h = hunk(&s, "tab\tx.txt", Side::Unstaged, 0);
        s.run(|r| apply_hunk(r, "tab\tx.txt", &h, HunkAction::Stage));
        assert_eq!(s.halves("tab\tx.txt").1, "2\n");
        assert_eq!(quoted("a/x\"y"), "\"a/x\\\"y\"");
    }

    #[test]
    fn crlf_files_diff_and_stage_as_git_stores_them() {
        let s = Scratch::new("crlf");
        s.write(".gitattributes", "*.bat text eol=crlf\n");
        s.commit("x.bat", "a\r\nb\r\nc\r\n");
        s.write("x.bat", "a\r\nB\r\nc\r\n");
        // One line changed, not every line.
        let h = hunk(&s, "x.bat", Side::Unstaged, 0);
        assert_eq!(h.lines.iter().filter(|l| l.op != Op::Context).count(), 2);
        s.run(|r| apply_hunk(r, "x.bat", &h, HunkAction::Stage));
        assert_eq!(s.halves("x.bat").1, "a\nB\nc\n");
    }

    #[test]
    fn hunks_keep_the_bytes_exactly() {
        let cases = [
            ("crlf.txt", "a\r\nb\r\nc\r\n", "a\r\nB\r\nc\r\n"),
            ("tabs.txt", "\tx\n\ty\n", "\tx\n\t\tY\n"),
            ("no-eol.txt", "a\nb", "a\nB"),
            ("adds-eol.txt", "a", "a\nb\n"),
            ("drops-eol.txt", "a\nb\n", "a\nb"),
            ("with space.txt", "1\n", "2\n"),
        ];
        let s = Scratch::new("bytes");
        for (path, old, _) in cases {
            s.write(path, old);
        }
        s.run(add_all);
        s.git(&["commit", "-q", "-m", "base"]);
        for (path, old, new) in cases {
            s.write(path, new);
            let h = hunk(&s, path, Side::Unstaged, 0);
            s.run(|r| apply_hunk(r, path, &h, HunkAction::Stage));
            assert_eq!(s.halves(path), (old.into(), new.into(), new.into()), "{path}");
        }
    }

    #[test]
    fn names_are_taken_literally() {
        let s = Scratch::new("literal");
        s.write("a*.txt", "star\n");
        s.write("ab.txt", "plain\n");
        s.run(|r| add(r, &["a*.txt".into()]));
        assert_eq!(s.halves("a*.txt").1, "star\n");
        // Not matched by the `*`.
        assert_eq!(s.halves("ab.txt").1, "");
    }

    #[test]
    fn reads_and_resolves_conflicts() {
        let s = Scratch::new("conflicts");
        s.write("both.txt", "base\n");
        s.write("they-del.txt", "base\n");
        s.write("we-del.txt", "base\n");
        s.git(&["add", "."]);
        s.git(&["commit", "-q", "-m", "base"]);
        s.git(&["switch", "-q", "-c", "side"]);
        s.write("both.txt", "side\n");
        s.write("we-del.txt", "side\n");
        s.git(&["rm", "-q", "they-del.txt"]);
        s.git(&["commit", "-q", "-a", "-m", "side"]);
        s.git(&["switch", "-q", "main"]);
        s.write("both.txt", "main\n");
        s.write("they-del.txt", "main\n");
        s.git(&["rm", "-q", "we-del.txt"]);
        s.git(&["commit", "-q", "-a", "-m", "main"]);
        // Fails: that's the conflict.
        let _ = crate::process::run("git", &args(&["merge", "-q", "side"]), Some(&s.0));

        let c = conflicts(&s.repo()).unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c["both.txt"], Conflict { base: true, ours: true, theirs: true });
        assert_eq!(c["they-del.txt"], Conflict { base: true, ours: true, theirs: false });
        assert_eq!(c["we-del.txt"], Conflict { base: true, ours: false, theirs: true });
        assert!(c["both.txt"].in_content() && !c["we-del.txt"].in_content());

        // Keep one, take the other's deletion.
        s.run(|r| add(r, &["they-del.txt".into()]));
        s.run(|r| remove(r, &["we-del.txt".into()]));
        let c = conflicts(&s.repo()).unwrap();
        assert_eq!(c.keys().collect::<Vec<_>>(), ["both.txt"]);
        assert_eq!(s.halves("they-del.txt").1, "main\n");
        assert!(!s.0.join("we-del.txt").exists());
    }
}
