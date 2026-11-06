//! `git blame`: which commit last changed each line of a file.

use std::collections::HashMap;
use std::path::Path;

/// One line of the file and the commit that last changed it.
pub struct Blamed {
    /// Full id; all zeros for a line not committed yet.
    pub id: String,
    pub text: String,
}

/// What porcelain says about a commit the first time it shows up.
#[derive(Default, Clone)]
pub struct Who {
    pub author: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub summary: String,
    /// The file in that commit, from the repository's top: an older name if it moved.
    pub path: String,
}

pub struct Blame {
    pub lines: Vec<Blamed>,
    pub commits: HashMap<String, Who>,
}

/// Blames `path` in `dir` (or here), as it is in the working tree or at `rev`. Nothing
/// on stdin, so it can run behind a full screen.
pub fn read(dir: Option<&Path>, rev: Option<&str>, path: &str) -> Result<Blame, String> {
    let args: Vec<String> =
        ["blame", "--porcelain"].into_iter().chain(rev).chain(["--", path]).map(String::from).collect();
    match crate::process::run_with_input("git", &args, dir, b"", &[]) {
        Ok(out) if out.ok() => Ok(parse(&out.stdout)),
        Ok(out) => Err(out.stderr.trim().to_string()),
        Err(e) => Err(format!("could not run git: {e}")),
    }
}

/// `--porcelain`: a `<id> <old> <new> [<count>]` line, the commit's fields the first
/// time it shows up, then the line itself after a tab.
fn parse(text: &str) -> Blame {
    let mut blame = Blame { lines: Vec::new(), commits: HashMap::new() };
    let mut id = String::new();
    for l in text.lines() {
        if let Some(line) = l.strip_prefix('\t') {
            blame.lines.push(Blamed { id: id.clone(), text: line.replace('\t', "    ") });
            continue;
        }
        let (key, value) = l.split_once(' ').unwrap_or((l, ""));
        if key.len() == 40 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
            id = key.to_string();
            continue;
        }
        let who = blame.commits.entry(id.clone()).or_default();
        match key {
            "author" => who.author = value.to_string(),
            "author-time" => who.time = value.parse().unwrap_or(0),
            "summary" => who.summary = value.to_string(),
            "filename" => who.path = value.to_string(),
            _ => {}
        }
    }
    blame
}

/// A line no commit has yet: still in the working tree or the index.
pub fn uncommitted(id: &str) -> bool {
    id.bytes().all(|b| b == b'0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::run::Scratch;

    #[test]
    fn each_line_has_its_commit() {
        let s = Scratch::new("blame");
        s.commit("a.txt", "one\ntwo\n");
        s.commit("a.txt", "one\n2\tx\n");
        s.write("a.txt", "one\n2\tx\nthree\n");
        let b = read(Some(&s.0), None, "a.txt").unwrap();
        let texts: Vec<&str> = b.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["one", "2    x", "three"]);
        assert_eq!(b.commits[&b.lines[0].id].path, "a.txt");
        // Two commits and the uncommitted line.
        assert_ne!(b.lines[0].id, b.lines[1].id);
        assert!(uncommitted(&b.lines[2].id));
        assert_eq!(b.commits[&b.lines[1].id].summary, "a.txt");
        assert!(b.commits[&b.lines[0].id].time > 0);
        assert!(read(Some(&s.0), None, "missing.txt").is_err());
        // At the first commit: its two lines, both from it.
        let old = read(Some(&s.0), Some("HEAD~1"), "a.txt").unwrap();
        assert_eq!(old.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        assert!(old.lines.iter().all(|l| l.id == b.lines[0].id));
    }
}
