//! `gg rebase`: git does the rebase; gg only replaces the todo-list editor.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::core::{self, todo};

/// Flags that control a rebase already in progress; these never get `-i` added.
const CONTROL: [&str; 6] = ["--continue", "--abort", "--skip", "--quit", "--edit-todo", "--show-current-patch"];

/// Runs `git rebase -i <args>` (or `git rebase --continue` etc.) with gg as the todo editor.
/// Returns git's exit code.
///
/// Shortcuts for choosing which commits to edit:
/// - no arguments: this branch's own commits (since it split off main);
/// - a number `N`: the last N commits;
/// - `-p` / `--pick`: choose the oldest commit to edit in fzf.
pub fn run(args: &[String]) -> Result<i32> {
    let control = args.iter().any(|a| CONTROL.contains(&a.as_str()));
    let (args, desc) = if control {
        (args.to_vec(), None)
    } else {
        match resolve(args)? {
            Some((args, desc)) => (args, desc),
            None => return Ok(0), // picker closed
        }
    };
    let interactive = args.iter().any(|a| a == "-i" || a == "--interactive");

    let exe = std::env::current_exe().context("can't find the gg binary")?;
    // Git runs this through the shell, so quote the path.
    let quoted = format!("'{}'", exe.display().to_string().replace('\'', r"'\''"));
    // When editing the list mid-rebase, quitting should keep it, not empty it.
    let editor = if args.iter().any(|a| a == "--edit-todo") {
        format!("{quoted} __rebase-editor --keep-on-quit")
    } else {
        format!("{quoted} __rebase-editor")
    };

    let mut cmd = Command::new("git");
    cmd.arg("rebase");
    if !control && !interactive {
        cmd.arg("-i");
    }
    cmd.args(&args).env("GIT_SEQUENCE_EDITOR", editor);
    // Tells the editor how the commits were chosen, for its header.
    match &desc {
        Some(d) => cmd.env(DESC_ENV, d),
        None => cmd.env_remove(DESC_ENV),
    };

    let marker = cancel_marker()?;
    let _ = std::fs::remove_file(&marker);
    let status = cmd.status().context("could not run git")?;
    if std::fs::remove_file(&marker).is_ok() {
        // The user quit the editor; git's "nothing to do" error is expected.
        eprintln!("Rebase cancelled. Nothing changed.");
        return Ok(0);
    }
    Ok(status.code().unwrap_or(1))
}

const DESC_ENV: &str = "GG_REBASE_DESC";

/// Turns gg's shortcuts into `git rebase` arguments plus a description for the header.
/// `None` means the user closed the picker.
fn resolve(args: &[String]) -> Result<Option<(Vec<String>, Option<String>)>> {
    let repo = core::open()?;
    let (first, rest) = match args.split_first() {
        Some((f, r)) => (Some(f.as_str()), r),
        None => (None, &[][..]),
    };
    let with_rest = |base: String| [vec![base], rest.to_vec()].concat();

    match first {
        None => match fork_point(&repo)?.or(unpushed(&repo)?) {
            Some((base, desc)) => Ok(Some((vec![base.to_string()], Some(desc)))),
            None => anyhow::bail!(
                "nothing to pick automatically: this branch has no commits of its own and nothing unpushed.\n\
                 Try `gg rebase 5` (last 5 commits) or `gg rebase -p` (choose where to start)."
            ),
        },
        Some("-p" | "--pick") => pick(&repo, rest),
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            let n: usize = n.parse().context("count is too large")?;
            if n == 0 {
                anyhow::bail!("nothing to rebase: 0 commits");
            }
            let head = repo.head_id().context("this branch has no commits yet")?.detach();
            let available = repo.rev_walk([head]).first_parent_only().all()?.take(n + 1).count();
            let desc = Some(format!("last {n} commit{}", if n == 1 { "" } else { "s" }));
            match available.cmp(&n) {
                std::cmp::Ordering::Greater => Ok(Some((with_rest(format!("HEAD~{n}")), desc))),
                // Exactly the whole history: git needs --root for that.
                std::cmp::Ordering::Equal => Ok(Some((with_rest("--root".into()), desc))),
                std::cmp::Ordering::Less => anyhow::bail!("this branch only has {available} commits"),
            }
        }
        Some(_) => Ok(Some((args.to_vec(), None))),
    }
}

/// Where this branch split off the default branch, and a description like
/// "4 commits since branching off main". Compares with both the local and origin
/// copies of main and keeps the closer one. `None` if there's nothing to rebase.
fn fork_point(repo: &gix::Repository) -> Result<Option<(gix::ObjectId, String)>> {
    let Ok(head) = repo.head_id() else { return Ok(None) };
    let head = head.detach();
    let Some((main, _)) = core::branches::default_branch(repo) else { return Ok(None) };
    // On main itself there's no fork; `unpushed` covers that case.
    if repo.head_name()?.is_some_and(|n| n.shorten() == main.as_str()) {
        return Ok(None);
    }

    let mut best: Option<(usize, gix::ObjectId, String)> = None;
    for name in [format!("refs/heads/{main}"), format!("refs/remotes/origin/{main}")] {
        let Ok(mut r) = repo.find_reference(name.as_str()) else { continue };
        let short = r.name().shorten().to_string();
        let Ok(tip) = r.peel_to_id() else { continue };
        let Ok(Some(base)) = repo.merge_base(head, tip.detach()) else { continue };
        let base = base.detach();
        let n = repo.rev_walk([head]).with_hidden([base]).all()?.count();
        if n > 0 && best.as_ref().is_none_or(|(m, ..)| n < *m) {
            best = Some((n, base, short));
        }
    }
    Ok(best.map(|(n, base, from)| {
        let s = if n == 1 { "" } else { "s" };
        (base, format!("{n} commit{s} since branching off {from}"))
    }))
}

/// Commits not yet pushed to this branch's upstream, e.g. on main before a push.
fn unpushed(repo: &gix::Repository) -> Result<Option<(gix::ObjectId, String)>> {
    let (Some(name), Ok(head)) = (repo.head_name()?, repo.head_id()) else { return Ok(None) };
    let Some(Ok(up)) = repo.branch_remote_tracking_ref_name(name.as_ref(), gix::remote::Direction::Fetch) else {
        return Ok(None);
    };
    let Ok(mut up_ref) = repo.find_reference(up.as_ref()) else { return Ok(None) };
    let head = head.detach();
    let Ok(Some(base)) = repo.merge_base(head, up_ref.peel_to_id()?.detach()) else { return Ok(None) };
    let base = base.detach();
    let n = repo.rev_walk([head]).with_hidden([base]).all()?.count();
    if n == 0 {
        return Ok(None);
    }
    let s = if n == 1 { "" } else { "s" };
    Ok(Some((base, format!("{n} unpushed commit{s} (not on {})", up.shorten()))))
}

/// Choose the oldest commit to edit from this branch's history (full-screen log, or fzf).
fn pick(repo: &gix::Repository, rest: &[String]) -> Result<Option<(Vec<String>, Option<String>)>> {
    #[cfg(feature = "tui")]
    let picked = {
        use crate::tui::log::{Mode, Options, run};
        run(repo, Options { mode: Mode::Pick, all: false, count: 5000 })?
    };
    #[cfg(not(feature = "tui"))]
    let picked = {
        let commits = core::log::read(repo, 5000, false)?;
        let lines = crate::render::log::lines(&commits, &crate::render::Paint { on: true });
        let label = format!("{} · rebase: choose the oldest commit to edit", core::repo_name(repo));
        crate::fzf::pick_commit(&lines, &label, "rebase from here")?
    };
    let Some(picked) = picked else {
        return Ok(None);
    };
    let commit = repo.rev_parse_single(picked.as_str())?.object()?.peel_to_commit()?;
    let subject = commit.message()?.summary().to_string();
    let desc = Some(format!("from {picked} {subject}"));
    let base = match commit.parent_ids().next() {
        Some(parent) => parent.to_string(),
        None => "--root".into(),
    };
    Ok(Some(([vec![base], rest.to_vec()].concat(), desc)))
}

/// Left by the editor when the user quits, so `run` can tell a cancel from a failure.
fn cancel_marker() -> Result<std::path::PathBuf> {
    Ok(core::open()?.path().join("gg-rebase-cancelled"))
}

/// Called by git with the todo file path. Writing it back applies the changes.
pub fn edit(file: &Path, keep_on_quit: bool) -> Result<()> {
    let repo = core::open()?;
    let lines = todo::read_file(file)?;
    let header = header(&repo, &lines);

    #[cfg(feature = "tui")]
    {
        use crate::tui::rebase::{Header, Outcome, run};
        let header = Header { branch: header.0, detail: header.1 };
        match run(&repo, lines, header, newest_first(&repo))? {
            Outcome::Apply(new) => std::fs::write(file, todo::render(&new))?,
            Outcome::Quit if keep_on_quit => {} // leave the file as git wrote it
            // An empty todo list makes git abort the rebase and leave the branch untouched.
            Outcome::Quit => {
                std::fs::write(file, "")?;
                std::fs::write(cancel_marker()?, "")?;
            }
        }
        Ok(())
    }
    #[cfg(not(feature = "tui"))]
    {
        let _ = (lines, header, keep_on_quit);
        anyhow::bail!("the rebase editor needs gg built with the `tui` feature")
    }
}

/// `git config gg.rebaseOrder oldest` shows git's order; anything else (or unset) shows newest first.
#[cfg(feature = "tui")]
fn newest_first(repo: &gix::Repository) -> bool {
    let order = repo.config_snapshot().string("gg.rebaseOrder");
    !order.is_some_and(|o| o.eq_ignore_ascii_case(b"oldest"))
}

/// Branch name and a short description of which commits are listed.
fn header(repo: &gix::Repository, lines: &[todo::Line]) -> (String, String) {
    let dir = repo.path().join("rebase-merge");
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string());
    let branch = read("head-name")
        .map(|h| h.trim_start_matches("refs/heads/").to_string())
        .unwrap_or_else(|| "HEAD".into());
    let onto = read("onto").map(|o| o.chars().take(7).collect::<String>()).unwrap_or_default();
    let n = lines.iter().filter(|l| matches!(l, todo::Line::Commit { .. })).count();
    let done = read("done").map(|d| todo::parse(&d).len()).unwrap_or(0);
    // "4 commits since branching off main", or "onto 1a2b3c4 · 4 commits".
    let detail = match std::env::var(DESC_ENV) {
        Ok(desc) if !desc.is_empty() => desc,
        _ => {
            let s = if n == 1 { "" } else { "s" };
            let mut d = format!("{n} commit{s}");
            if !onto.is_empty() {
                d = format!("onto {onto} · {d}");
            }
            if done > 0 {
                d += &format!(" left · {done} done");
            }
            d
        }
    };
    (branch, detail)
}
