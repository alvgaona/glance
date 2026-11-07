//! `git diff` for the diff screen: what to compare, which files changed, and one file's
//! patch. Runs in the folder glance was started from, so paths the user typed mean what
//! they meant to git.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::diff::{PatchLine, Size};
use super::run::{args, top};
use super::status::ChangeKind;
use super::tree::FileStat;

/// What the diff is between.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    /// Working tree vs index: `git diff`.
    Unstaged,
    /// Index vs HEAD: `git diff --staged`.
    Staged,
    /// Working tree vs HEAD: `git diff HEAD`.
    Head,
    /// Anything else the user typed (`main..`, `-- src/`), as given.
    Args(Vec<String>),
    /// One commit against its first parent: `glance show`.
    Commit { id: String, parent: String, subject: String },
}

impl Target {
    pub fn from_args(a: &[String]) -> Self {
        match a {
            [] => Target::Unstaged,
            [s] if s == "--staged" || s == "--cached" => Target::Staged,
            [h] if h == "HEAD" => Target::Head,
            _ => Target::Args(a.to_vec()),
        }
    }

    fn args(&self) -> Vec<String> {
        match self {
            Target::Unstaged => Vec::new(),
            Target::Staged => args(&["--staged"]),
            Target::Head => args(&["HEAD"]),
            Target::Args(a) => a.clone(),
            Target::Commit { id, parent, .. } => vec![parent.clone(), id.clone()],
        }
    }

    /// The user's args without their paths (`main.. -- src/` → `main..`): for one file,
    /// that file is the only path.
    fn revs(&self) -> Vec<String> {
        self.args().into_iter().take_while(|a| a != "--").collect()
    }

    pub fn label(&self) -> String {
        match self {
            Target::Unstaged => "unstaged".into(),
            Target::Staged => "staged".into(),
            Target::Head => "since HEAD".into(),
            Target::Args(a) => a.join(" "),
            Target::Commit { id, subject, .. } => format!("{} {subject}", &id[..7.min(id.len())]),
        }
    }
}

/// `rev` as a [`Target::Commit`]. A first commit is compared with the empty tree.
pub fn commit(repo: &gix::Repository, rev: &str) -> Result<Target> {
    let id = repo.rev_parse_single(rev).with_context(|| format!("unknown revision `{rev}`"))?;
    let c = id.object()?.peel_to_commit().with_context(|| format!("`{rev}` is not a commit"))?;
    let parent = c.parent_ids().next().map_or_else(|| repo.empty_tree().id, |p| p.detach());
    let subject = c.message()?.summary().to_string();
    Ok(Target::Commit { id: c.id.to_string(), parent: parent.to_string(), subject })
}

/// How each file's diff is cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cut {
    /// Ignore whitespace changes (`-w`).
    pub whitespace: bool,
    /// Lines of context around each change.
    pub context: u32,
    /// The whole file, not just the changes.
    pub full: bool,
}

impl Default for Cut {
    fn default() -> Self {
        Cut { whitespace: false, context: 3, full: false }
    }
}

/// `git diff` with options that make the output the same whatever the user's config.
fn base(whitespace: bool) -> Vec<String> {
    let mut a = args(&["diff", "--no-color", "--no-ext-diff", "-M"]);
    if whitespace {
        a.push("-w".into());
    }
    a
}

/// git's stdout (not trimmed: a patch's spacing matters), or its complaint.
fn stdout(dir: Option<&Path>, a: &[String]) -> Result<String> {
    let out = crate::process::run("git", a, dir)?;
    if !out.ok() {
        bail!("{}", out.stderr.trim());
    }
    Ok(out.stdout)
}

/// The changed files, in git's order, with lines added and removed.
pub fn files(dir: Option<&Path>, target: &Target, whitespace: bool) -> Result<Vec<FileStat>> {
    let run = |flag: &str| stdout(dir, &[base(whitespace), args(&["-z", flag]), target.args()].concat());
    let mut files = parse(&run("--name-status")?, &run("--numstat")?);
    // With -w, --name-status still names files whose only change is whitespace;
    // --numstat leaves them out.
    if whitespace {
        files.retain(|f| f.size.is_some());
    }
    Ok(files)
}

/// How many new files git doesn't track (and doesn't ignore), in the whole repo.
pub fn untracked(dir: Option<&Path>) -> usize {
    let a = args(&["ls-files", "--others", "--exclude-standard", "-z", "--", ":/"]);
    stdout(dir, &a).map_or(0, |s| s.split('\0').filter(|p| !p.is_empty()).count())
}

/// `--name-status -z` and `--numstat -z` joined: kind and names from the first, sizes
/// from the second.
fn parse(names: &str, nums: &str) -> Vec<FileStat> {
    let mut sizes = std::collections::HashMap::new();
    let mut t = nums.split('\0');
    while let Some(row) = t.next().filter(|r| !r.is_empty()) {
        let mut cols = row.splitn(3, '\t');
        let (added, removed, path) = (cols.next(), cols.next(), cols.next().unwrap_or_default());
        // A rename has an empty path here, then old and new as their own fields.
        let path = if path.is_empty() { t.nth(1).unwrap_or_default() } else { path };
        let size = match (added.and_then(|a| a.parse().ok()), removed.and_then(|r| r.parse().ok())) {
            (Some(added), Some(removed)) => Size::Lines { added, removed },
            _ => Size::Binary,
        };
        sizes.insert(path.to_string(), size);
    }

    let mut out = Vec::new();
    let mut t = names.split('\0');
    while let Some(status) = t.next().filter(|s| !s.is_empty()) {
        let kind = match status.as_bytes()[0] {
            b'A' => ChangeKind::Added,
            b'D' => ChangeKind::Deleted,
            b'R' => ChangeKind::Renamed,
            b'C' => ChangeKind::Copied,
            b'T' => ChangeKind::TypeChange,
            _ => ChangeKind::Modified,
        };
        let from =
            matches!(kind, ChangeKind::Renamed | ChangeKind::Copied).then(|| t.next().unwrap_or_default().to_string());
        let path = t.next().unwrap_or_default().to_string();
        out.push(FileStat { kind, size: sizes.get(&path).copied(), path, from, old: None, new: None });
    }
    out
}

/// One file's `git diff` text, cut as asked.
pub fn patch(dir: Option<&Path>, target: &Target, f: &FileStat, cut: Cut) -> Result<String> {
    let mut a = base(cut.whitespace);
    // As much context as the file has: the whole file.
    a.push(format!("-U{}", if cut.full { 1_000_000 } else { cut.context }));
    a.extend(target.revs());
    a.push("--".into());
    // Both names of a rename, so git pairs them up again.
    a.extend(f.from.iter().chain([&f.path]).map(|p| top(p)));
    stdout(dir, &a)
}

/// Each hunk's first line in the new file, and what git says it's in: the text after
/// `@@ … @@`, usually the function or section.
pub fn headers(patch: &str) -> Vec<(u32, String)> {
    patch
        .lines()
        .filter_map(|l| l.strip_prefix("@@ -"))
        .map(|h| {
            let (ranges, context) = h.split_once(" @@").unwrap_or((h, ""));
            let new = ranges.split(" +").nth(1).unwrap_or_default();
            let line = new.split(',').next().and_then(|n| n.parse().ok()).unwrap_or(1);
            (line, context.trim().replace('\t', "    "))
        })
        .collect()
}

/// A commit that changed a file.
#[derive(Debug, Clone)]
pub struct Touch {
    pub id: String,
    pub author: String,
    pub time: i64,
    pub subject: String,
}

/// The last `n` commits that changed `path` (following renames), up to `from` or HEAD.
pub fn history(dir: Option<&Path>, from: Option<&str>, path: &str, n: usize) -> Vec<Touch> {
    let mut a = args(&["log", &format!("-{n}"), "--follow", "--format=%h%x00%an%x00%at%x00%s"]);
    a.extend(from.map(String::from));
    a.extend(["--".into(), top(path)]);
    let Ok(out) = stdout(dir, &a) else { return Vec::new() };
    out.lines()
        .filter_map(|l| {
            let mut f = l.splitn(4, '\0');
            let (id, author, time) = (f.next()?, f.next()?, f.next()?.parse().ok()?);
            Some(Touch { id: id.into(), author: author.into(), time, subject: f.next()?.into() })
        })
        .collect()
}

/// A patch's lines for drawing: headers dropped, line numbers counted
/// from each `@@ -a,b +c,d @@`, tabs expanded.
pub fn lines(patch: &str) -> Vec<PatchLine> {
    let (mut old, mut new) = (0, 0);
    let mut out = Vec::new();
    let mut in_hunk = false;
    for l in patch.lines() {
        let text = |t: &str| t.replace('\t', "    ");
        if let Some(h) = l.strip_prefix("@@ -") {
            let start = |s: &str| s.split(',').next().and_then(|n| n.parse().ok()).unwrap_or(1);
            let mut sides = h.split(' ');
            old = start(sides.next().unwrap_or_default());
            new = start(sides.next().unwrap_or_default().trim_start_matches('+'));
            out.push(PatchLine::Hunk { old_start: old, new_start: new });
            in_hunk = true;
        } else if !in_hunk {
            // `diff --git`, `index`, `---`, `+++` and the like.
        } else if let Some(t) = l.strip_prefix('+') {
            out.push(PatchLine::Added { new, text: text(t) });
            new += 1;
        } else if let Some(t) = l.strip_prefix('-') {
            out.push(PatchLine::Removed { old, text: text(t) });
            old += 1;
        } else if let Some(t) = l.strip_prefix(' ') {
            out.push(PatchLine::Context { old, new, text: text(t) });
            old += 1;
            new += 1;
        } else if l.starts_with("diff ") {
            in_hunk = false;
        }
        // `\ No newline at end of file` is skipped.
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::run::Scratch;

    #[test]
    fn reads_the_args_as_a_target() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(Target::from_args(&[]), Target::Unstaged);
        assert_eq!(Target::from_args(&s(&["--cached"])), Target::Staged);
        assert_eq!(Target::from_args(&s(&["HEAD"])), Target::Head);
        let t = Target::from_args(&s(&["main..", "--", "src/"]));
        assert_eq!(t.revs(), ["main.."]);
    }

    #[test]
    fn lists_changes_renames_and_binaries() {
        let s = Scratch::new("compare-files");
        s.commit("a.txt", "1\n2\n");
        s.commit("old.txt", "same\n");
        s.write("a.txt", "1\n2\n3\n");
        s.git(&["mv", "old.txt", "new name.txt"]);
        s.write("bin.dat", "a\0b");
        s.git(&["add", "bin.dat"]);
        let dir = Some(s.0.as_path());

        let unstaged = files(dir, &Target::Unstaged, false).unwrap();
        assert_eq!(unstaged.len(), 1);
        assert_eq!(unstaged[0].size, Some(Size::Lines { added: 1, removed: 0 }));

        let head = files(dir, &Target::Head, false).unwrap();
        let by = |p: &str| head.iter().find(|f| f.path == p).unwrap();
        assert_eq!(by("new name.txt").kind, ChangeKind::Renamed);
        assert_eq!(by("new name.txt").from.as_deref(), Some("old.txt"));
        assert_eq!(by("bin.dat").size, Some(Size::Binary));

        let p = patch(dir, &Target::Head, by("new name.txt"), Cut::default()).unwrap();
        assert!(p.contains("rename from old.txt"), "{p}");
        let p = patch(dir, &Target::Unstaged, &unstaged[0], Cut { full: true, ..Cut::default() }).unwrap();
        assert!(p.ends_with("+3\n"), "{p}");

        // Only whitespace: gone with -w.
        s.write("new name.txt", "same \n");
        let ws = files(dir, &Target::Unstaged, true).unwrap();
        assert_eq!(ws.len(), 1, "{ws:?}");

        let h = history(dir, None, "a.txt", 5);
        assert_eq!(h.iter().map(|t| t.subject.as_str()).collect::<Vec<_>>(), ["a.txt"]);

        s.write("loose.txt", "x\n");
        assert_eq!(untracked(dir), 1);
        // The first commit, against nothing.
        let repo = s.repo();
        let first = commit(&repo, "HEAD~1").unwrap();
        assert!(matches!(&first, Target::Commit { subject, .. } if subject.contains("a.txt")), "{first:?}");
        let added = files(dir, &first, false).unwrap();
        assert_eq!(added[0].kind, ChangeKind::Added);
    }

    #[test]
    fn reads_hunk_headers() {
        let p = "--- a/x\n+++ b/x\n@@ -1,2 +1,3 @@\n a\n@@ -40 +41,2 @@ impl Store {\n b\n";
        assert_eq!(headers(p), [(1, String::new()), (41, "impl Store {".into())]);
    }

    #[test]
    fn numbers_patch_lines() {
        let p = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -4,3 +4,3 @@ fn x\n a\n-b\n+\tB\n c\n\\ No newline at end of file\n";
        let got: Vec<String> = lines(p)
            .into_iter()
            .map(|l| match l {
                PatchLine::Hunk { old_start, new_start } => format!("@{old_start},{new_start}"),
                PatchLine::Context { old, new, text } => format!("{old},{new} {text}"),
                PatchLine::Removed { old, text } => format!("{old}- {text}"),
                PatchLine::Added { new, text } => format!("{new}+ {text}"),
            })
            .collect();
        assert_eq!(got, ["@4,4", "4,4 a", "5- b", "5+     B", "6,6 c"]);
    }
}
