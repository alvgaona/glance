//! Remotes: read them (URLs, their branches, who tracks them, when they were last
//! fetched) and change them through `git remote`.

use std::path::Path;
use std::time::SystemTime;

use anyhow::Result;
use gix::bstr::{BStr, ByteSlice};

use super::run::{self, Ran, args, git};

pub struct Remote {
    pub name: String,
    /// As written in the config (`remote.<name>.url`), before any `insteadOf`.
    pub url: Option<String>,
    /// Only when set apart from `url` (`remote.<name>.pushurl`).
    pub push_url: Option<String>,
    /// Its default branch, from `refs/remotes/<name>/HEAD`.
    pub head: Option<String>,
    /// Its branches as last fetched, newest commit first.
    pub branches: Vec<RemoteBranch>,
    /// Local branches whose upstream is on this remote.
    pub tracking: Vec<String>,
    /// When it was last fetched (or its branches last changed), seconds since the epoch.
    pub fetched: Option<i64>,
}

pub struct RemoteBranch {
    /// Without the remote: `main`, `feat/x`.
    pub name: String,
    pub id: String,
    pub time: i64,
    pub summary: String,
}

/// Every remote, by name.
pub fn list(repo: &gix::Repository) -> Result<Vec<Remote>> {
    let names: Vec<String> = repo.remote_names().into_iter().map(|n| n.to_string()).collect();
    let config = repo.config_snapshot();
    let config = config.plumbing();
    let get = |section: &str, sub: &str, key: &str| {
        config.string_by(section, Some(AsRef::<BStr>::as_ref(sub)), key).map(|v| v.to_str_lossy().into_owned())
    };

    // Which remote each local branch tracks.
    let mut upstreams: Vec<(String, String)> = Vec::new();
    for r in repo.references()?.local_branches()?.flatten() {
        let b = r.name().shorten().to_string();
        if let Some(remote) = get("branch", &b, "remote") {
            upstreams.push((remote, b));
        }
    }

    // Remote branches, each given to the longest remote name that prefixes it.
    let mut by_remote: Vec<Vec<RemoteBranch>> = names.iter().map(|_| Vec::new()).collect();
    let mut heads: Vec<Option<String>> = names.iter().map(|_| None).collect();
    for r in repo.references()?.remote_branches()?.flatten() {
        let mut r = r;
        let short = r.name().shorten().to_string();
        let Some((i, rest)) = names
            .iter()
            .enumerate()
            .filter_map(|(i, n)| short.strip_prefix(&format!("{n}/")).map(|rest| (i, rest.to_string())))
            .max_by_key(|(i, _)| names[*i].len())
        else {
            continue;
        };
        if rest == "HEAD" {
            if let gix::refs::TargetRef::Symbolic(t) = r.target() {
                let prefix = format!("refs/remotes/{}/", names[i]);
                heads[i] = Some(t.as_bstr().to_str_lossy().trim_start_matches(&prefix).to_string());
            }
            continue;
        }
        let Ok(tip) = r.peel_to_id() else { continue };
        let Ok(commit) = repo.find_commit(tip.detach()) else { continue };
        by_remote[i].push(RemoteBranch {
            name: rest,
            id: commit.id().shorten_or_id().to_string(),
            time: commit.time().map(|t| t.seconds).unwrap_or(0),
            summary: commit.message().map(|m| m.summary().to_str_lossy().into_owned()).unwrap_or_default(),
        });
    }

    let fetch_head = fetch_head(repo);
    let mut out = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let url = get("remote", name, "url");
        let push_url = get("remote", name, "pushurl").filter(|p| Some(p) != url.as_ref());
        let mut branches = std::mem::take(&mut by_remote[i]);
        branches.sort_by_key(|b| std::cmp::Reverse(b.time));
        let from_fetch_head = match (&fetch_head, &url) {
            (Some((time, text)), Some(u)) if mentions(text, u) => Some(*time),
            _ => None,
        };
        let from_logs = newest_file(&repo.common_dir().join("logs/refs/remotes").join(name));
        out.push(Remote {
            fetched: from_fetch_head.max(from_logs),
            tracking: upstreams.iter().filter(|(r, _)| r == name).map(|(_, b)| b.clone()).collect(),
            head: heads[i].take(),
            name: name.clone(),
            url,
            push_url,
            branches,
        });
    }
    Ok(out)
}

/// FETCH_HEAD's time and text. It names the URL of every remote the last fetch read.
fn fetch_head(repo: &gix::Repository) -> Option<(i64, String)> {
    [repo.git_dir(), repo.common_dir()].iter().find_map(|d| {
        let p = d.join("FETCH_HEAD");
        Some((mtime(&p)?, std::fs::read_to_string(&p).ok()?))
    })
}

/// Does FETCH_HEAD's text come from `url`? Its lines end in "… of <url>", with the user
/// and a trailing `.git` taken off.
fn mentions(fetch_head: &str, url: &str) -> bool {
    let want = anonymous(url);
    fetch_head.lines().filter_map(|l| l.rsplit_once(" of ")).any(|(_, u)| anonymous(u) == want)
}

/// `https://me@host/a/b.git` → `https://host/a/b`; `git@host:a/b.git` → `host:a/b`.
fn anonymous(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (format!("{s}://"), r),
        None => (String::new(), url),
    };
    // The user is whatever comes before an `@` in the host part.
    let host_end = rest.find(['/', ':']).unwrap_or(rest.len());
    let rest = match rest[..host_end].rfind('@') {
        Some(at) => &rest[at + 1..],
        None => rest,
    };
    format!("{scheme}{rest}")
}

fn mtime(p: &Path) -> Option<i64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs() as i64)
}

/// The newest file under `dir`: when any of the remote's branches last moved.
fn newest_file(dir: &Path) -> Option<i64> {
    let mut newest = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let t = if p.is_dir() { newest_file(&p) } else { mtime(&p) };
        newest = newest.max(t);
    }
    newest
}

pub fn add(name: &str, url: &str) -> Ran {
    git(&args(&["remote", "add", name, url]))
}

pub fn remove(name: &str) -> Ran {
    git(&args(&["remote", "remove", name]))
}

/// Also moves its remote branches and the upstream of every branch tracking it.
pub fn rename(old: &str, new: &str) -> Ran {
    git(&args(&["remote", "rename", old, new]))
}

pub fn set_url(name: &str, url: &str) -> Ran {
    git(&args(&["remote", "set-url", name, url]))
}

/// A push URL apart from the fetch one; `None` pushes to the fetch URL again.
pub fn set_push_url(name: &str, url: Option<&str>) -> Ran {
    match url {
        Some(u) => git(&args(&["remote", "set-url", "--push", name, u])),
        None => git(&args(&["config", "--unset-all", &format!("remote.{name}.pushurl")])),
    }
}

/// `git fetch --prune` of one remote, or of all of them.
pub fn fetch_args(name: Option<&str>) -> Vec<String> {
    match name {
        Some(n) => args(&["fetch", "--prune", n]),
        None => args(&["fetch", "--all", "--prune"]),
    }
}

/// Asks the remote for its default branch and points `<name>/HEAD` at it.
pub fn set_head_args(name: &str) -> Vec<String> {
    args(&["remote", "set-head", name, "--auto"])
}

/// A git command that talks to remotes but never prompts: it runs behind a full screen,
/// so a remote that needs a password fails and says so instead. Returns the job to run
/// on another thread; the repository is only read here, on this one.
pub fn no_prompt(repo: &gix::Repository, args: Vec<String>) -> impl FnOnce() -> Ran + Send + 'static {
    // Keep ssh from asking for a passphrase too, unless the user set their own ssh command.
    let own_ssh =
        std::env::var_os("GIT_SSH_COMMAND").is_some() || repo.config_snapshot().string("core.sshCommand").is_some();
    move || {
        let mut env = vec![("GIT_TERMINAL_PROMPT", "0")];
        if !own_ssh {
            env.push(("GIT_SSH_COMMAND", "ssh -o BatchMode=yes"));
        }
        run::git_no_stdin(&args, &env)
    }
}

/// What a fetch did, from what it printed.
#[derive(Debug, Default, PartialEq)]
pub struct Fetched {
    pub new: usize,
    pub updated: usize,
    pub pruned: usize,
}

impl Fetched {
    pub fn read(out: &str) -> Self {
        let mut f = Fetched::default();
        for l in out.lines().map(str::trim_start) {
            if l.starts_with("* [new") {
                f.new += 1;
            } else if l.starts_with("- [deleted]") {
                f.pruned += 1;
            } else if l.starts_with('+') || l.split_whitespace().next().is_some_and(|w| w.contains("..")) {
                f.updated += 1;
            }
        }
        f
    }
}

/// git's complaints in fetch output, without the "Fetching origin" / "From …" chatter.
pub fn errors(out: &str) -> String {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Fetching ") && !l.starts_with("From "))
        .filter(|l| !l.starts_with("* [") && !l.starts_with("- [") && !l.contains(" -> "))
        .take(2)
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_without_the_user() {
        assert_eq!(anonymous("https://me@github.com/a/b.git"), "https://github.com/a/b");
        assert_eq!(anonymous("git@github.com:a/b.git"), "github.com:a/b");
        assert_eq!(anonymous("ssh://git@host:22/a/b/"), "ssh://host:22/a/b");
        assert_eq!(anonymous("../up"), "../up");
        assert!(mentions("abc\t\tbranch 'main' of github.com:a/b\n", "git@github.com:a/b.git"));
        assert!(!mentions("abc\t\tbranch 'main' of github.com:a/c\n", "git@github.com:a/b.git"));
    }

    #[test]
    fn reads_what_a_fetch_did() {
        let out = "Fetching origin\nFrom ../up\n - [deleted]         (none)     -> origin/feat\n * [new branch]      new        -> origin/new\n   1a2b3c4..5d6e7f8  main       -> origin/main\n + 1a2b3c4...5d6e7f8 wip        -> origin/wip  (forced update)\n";
        assert_eq!(Fetched::read(out), Fetched { new: 1, updated: 2, pruned: 1 });
        assert_eq!(errors("Fetching origin\nfatal: could not read Username\n"), "fatal: could not read Username");
    }

    #[test]
    fn lists_remotes_with_their_branches() {
        let dir = std::env::temp_dir().join(format!("gg-remote-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sh = |cwd: &Path, a: &[&str]| run::test_git(cwd, a);
        sh(&dir, &["init", "-q", "-b", "main", "up"]);
        let up = dir.join("up");
        sh(&up, &["commit", "-q", "--allow-empty", "-m", "one"]);
        sh(&up, &["branch", "feat"]);
        sh(&dir, &["clone", "-q", "up", "down"]);
        let down = dir.join("down");
        sh(&down, &["remote", "add", "other", "../up"]);
        sh(&down, &["remote", "set-url", "--push", "other", "../elsewhere"]);
        sh(&down, &["fetch", "-q", "other"]);

        let repo = gix::open(&down).unwrap();
        let remotes = list(&repo).unwrap();
        let names: Vec<&str> = remotes.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["origin", "other"]);
        let (origin, other) = (&remotes[0], &remotes[1]);
        assert_eq!(origin.head.as_deref(), Some("main"));
        assert_eq!(origin.tracking, ["main"]);
        assert_eq!(origin.push_url, None);
        let mut b: Vec<&str> = origin.branches.iter().map(|b| b.name.as_str()).collect();
        b.sort();
        assert_eq!(b, ["feat", "main"]);
        assert_eq!(other.branches.len(), 2);
        assert!(other.tracking.is_empty());
        assert_eq!(other.push_url.as_deref(), Some("../elsewhere"));
        assert!(other.fetched.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
