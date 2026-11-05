//! Pull requests from gh, loaded in the background and matched to branches.

use super::*;

impl App<'_> {
    /// Asks gh for PRs on a background thread; `poll_prs` picks up the answer.
    pub(super) fn load_prs(&mut self) {
        let Some(dir) = self.repo.workdir().map(|d| d.to_path_buf()) else { return };
        if !github::available(self.repo) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(github::fetch(&dir));
        });
        self.prs = Prs::Loading(rx);
    }

    pub(super) fn poll_prs(&mut self) {
        let Prs::Loading(rx) = &self.prs else { return };
        let got = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(anyhow::anyhow!("gh stopped")),
        };
        self.prs = match got {
            Ok(list) => Prs::Ready(list),
            Err(e) => {
                self.toast = Some(Toast::new(format!("Couldn't load pull requests: {e}"), theme::YELLOW));
                Prs::Failed
            }
        };
        self.index_prs();
        self.cache.clear();
    }

    pub(super) fn index_prs(&mut self) {
        self.pr_of.clear();
        let Prs::Ready(list) = &self.prs else { return };
        let main = self.data.default.as_ref().map(|(n, _)| n.as_str());
        for b in &self.data.list {
            if let Some(pr) = github::find(list, b, &self.owners, main) {
                let i = list.iter().position(|p| std::ptr::eq(p, pr)).unwrap_or(0);
                self.pr_of.insert(b.name.clone(), i);
            }
        }
    }

    pub(super) fn pr(&self, b: &Branch) -> Option<&Pr> {
        let Prs::Ready(list) = &self.prs else { return None };
        self.pr_of.get(&b.name).map(|&i| &list[i])
    }

    /// Merged on GitHub (squash or rebase) with nothing added since: safe to delete
    /// even though git doesn't see it in main.
    pub(super) fn merged_by_pr(&self, b: &Branch) -> Option<&Pr> {
        self.pr(b).filter(|p| p.state == github::State::Merged && p.head == Some(b.oid))
    }
}
