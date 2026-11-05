//! Deleting branches, here and on remotes: the confirmations and the report afterwards.

use super::*;

impl App<'_> {
    pub(super) fn ask_delete_local(&mut self) {
        let all = self.targets();
        if all.iter().any(|b| b.remote) && all.len() == 1 {
            self.toast = Some(Toast::new("That's a remote branch: D deletes it on the remote", theme::YELLOW));
            return;
        }
        let (keep, list): (Vec<Branch>, Vec<Branch>) =
            all.into_iter().filter(|b| !b.remote).partition(|b| self.protected(b));
        if list.is_empty() {
            let why = match keep.first() {
                Some(b) if b.current => {
                    format!("{} is checked out; switch to another branch first", b.name)
                }
                Some(Branch { name, worktree: Some(path), .. }) => {
                    format!("{name} is checked out in the worktree at {}; remove that worktree first", short_path(path))
                }
                Some(b) => format!("{} is the main branch", b.name),
                None => "Nothing to delete".into(),
            };
            self.toast = Some(Toast::new(why, theme::YELLOW));
            return;
        }
        let main = self.default_name().unwrap_or("main").to_string();
        let only_here = |b: &Branch| branches::only_here(self.repo, b);

        let (title, body) = if list.len() == 1 {
            let b = &list[0];
            let n = only_here(b);
            let mut body = if n == 0 && b.merged {
                format!("It's merged into {main}, so no work is lost.")
            } else if let Some(pr) = self.merged_by_pr(b) {
                format!("PR #{} was merged into {main} (as new commits), so no work is lost.", pr.number)
            } else if n == 0 {
                "Every commit is also on another branch, so no work is lost.".to_string()
            } else {
                format!(
                    "{n} commit{} on this branch {} nowhere else.\nTo bring it back later:\n$ git branch {} {}",
                    if n == 1 { "" } else { "s" },
                    if n == 1 { "exists" } else { "exist" },
                    b.name,
                    b.id
                )
            };
            if b.gone() {
                body += "\nIts remote branch is already gone.";
            }
            (format!("Delete branch {}?", b.name), body)
        } else {
            let mut body = String::new();
            let mut unmerged = 0;
            for b in list.iter().take(10) {
                let n = only_here(b);
                let note = if n == 0 {
                    if b.merged { "merged" } else { "safe" }.to_string()
                } else if let Some(pr) = self.merged_by_pr(b) {
                    format!("PR #{} merged", pr.number)
                } else {
                    unmerged += 1;
                    format!("{n} commit{} only here", if n == 1 { "" } else { "s" })
                };
                body += &format!("$ {}  {}  ({note})\n", b.name, b.id);
            }
            if list.len() > 10 {
                body += &format!("…and {} more\n", list.len() - 10);
            }
            if unmerged > 0 {
                body += &format!(
                    "{unmerged} of them have commits no other branch has. `git branch <name> <id>` brings one back."
                );
            } else {
                body += "No work is lost: every commit is on another branch or in a merged PR.";
            }
            (format!("Delete {} branches?", list.len()), body.trim_end().to_string())
        };
        let mut body = body;
        if !keep.is_empty() {
            let why = |b: &Branch| match &b.worktree {
                _ if b.current => "checked out".to_string(),
                Some(p) => format!("in worktree {}", short_path(p)),
                None => "main branch".to_string(),
            };
            let names: Vec<String> = keep.iter().map(|b| format!("{} ({})", b.name, why(b))).collect();
            body += &format!("\nSkipping {}.", names.join(", "));
        }
        let danger = list.iter().any(|b| only_here(b) > 0 && self.merged_by_pr(b).is_none());
        self.confirm =
            Some((Confirm { title, body, yes: "Delete".into(), no: "Cancel".into(), danger }, Pending::Local(list)));
    }

    /// `git push <remote> --delete <branch>` for each pair.
    pub(super) fn remote_deletes(&mut self, list: Vec<(String, String)>) -> Vec<Vec<String>> {
        self.last_deleted = list.first().map(|(r, b)| format!("{b} on {r}")).unwrap_or_default();
        list.iter().map(|(remote, branch)| branches::delete_remote_args(remote, branch)).collect()
    }

    pub(super) fn ask_delete_remote(&mut self) {
        let main = self.default_name().map(str::to_string);
        let picked = self.targets();
        let mut items: Vec<PickItem> = Vec::new();
        for b in &picked {
            for (remote, branch) in b.remote_targets() {
                if Some(&branch) == main.as_ref() {
                    continue;
                }
                let note = match b.copies.iter().find(|c| c.remote == remote && c.branch == branch) {
                    Some(c) => sync_note(branches::ahead_behind(self.repo, b.oid, c.oid)),
                    None => String::new(),
                };
                items.push(PickItem { remote, branch, note, on: true });
            }
        }
        if items.is_empty() {
            self.toast = Some(Toast::new("Not on any remote", theme::YELLOW));
            return;
        }
        if items.len() > 1 {
            let title = if picked.len() == 1 {
                format!("Delete {} on which remotes?", picked[0].name)
            } else {
                format!("Delete {} branches on which remotes?", picked.len())
            };
            self.pick = Some(RemotePick { title, items, cursor: 0 });
            return;
        }
        let targets: Vec<(String, String)> = items.into_iter().map(|i| (i.remote, i.branch)).collect();
        let mut body: String = targets.iter().map(|(r, b)| format!("$ git push {r} --delete {b}\n")).collect();
        body += "This removes it for everyone using the remote. Local branches stay.";
        let title = if targets.len() == 1 {
            format!("Delete {}/{} on the remote?", targets[0].0, targets[0].1)
        } else {
            format!("Delete {} branches on the remote?", targets.len())
        };
        self.confirm = Some((
            Confirm { title, body, yes: "Delete on remote".into(), no: "Cancel".into(), danger: true },
            Pending::Remote(targets),
        ));
    }

    /// Toast summarising what the git commands did.
    pub(super) fn report(&mut self, results: &[(bool, String)]) {
        let failed: Vec<&String> = results.iter().filter(|(ok, _)| !ok).map(|(_, out)| out).collect();
        self.toast = Some(if failed.is_empty() {
            let n = results.len();
            Toast::new(
                if n == 1 { format!("Deleted {}", self.last_deleted) } else { format!("Deleted {n} branches") },
                theme::GREEN,
            )
        } else {
            Toast::new(format!("{} failed: {}", failed.len(), first_lines(failed[0])), theme::RED)
        });
    }
}
