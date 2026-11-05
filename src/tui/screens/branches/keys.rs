//! Keys and palette commands of the branch screen.

use super::*;

impl App<'_> {
    pub(super) fn on_key(&mut self, k: KeyEvent) -> Result<Option<Effect>> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        if let Some((input, from)) = &mut self.rename {
            match input.key(k) {
                Some(InputEvent::Cancel) => self.rename = None,
                Some(InputEvent::Done(to)) => {
                    let from = from.clone();
                    self.rename = None;
                    if !to.is_empty() && to != from {
                        let (ok, out) = branches::rename(&from, &to);
                        if ok {
                            self.toast = Some(Toast::new(format!("Renamed {from} → {to}"), theme::GREEN));
                            if self.marked.remove(&from) {
                                self.marked.insert(to.clone());
                            }
                            self.reload_selecting(Some(to))?;
                        } else {
                            self.toast = Some(Toast::new(first_lines(&out), theme::RED));
                        }
                    }
                }
                None => {}
            }
            return Ok(None);
        }
        if ctrl && k.code == KeyCode::Char('c') {
            return Ok(Some(Effect::Quit));
        }
        if let Some(p) = &mut self.pick {
            let last = p.items.len().saturating_sub(1);
            match k.code {
                KeyCode::Down | KeyCode::Char('j') => p.cursor = (p.cursor + 1).min(last),
                KeyCode::Up | KeyCode::Char('k') => p.cursor = p.cursor.saturating_sub(1),
                KeyCode::Char(' ') => {
                    p.items[p.cursor].on = !p.items[p.cursor].on;
                }
                KeyCode::Char('a') => {
                    let all = p.items.iter().all(|i| i.on);
                    p.items.iter_mut().for_each(|i| i.on = !all);
                }
                KeyCode::Enter => {
                    let chosen = p.chosen();
                    if chosen.is_empty() {
                        self.toast = Some(Toast::new("Nothing selected", theme::YELLOW));
                        return Ok(None);
                    }
                    self.pick = None;
                    self.marked.clear();
                    return Ok(Some(Effect::InTerminal(self.remote_deletes(chosen))));
                }
                KeyCode::Esc | KeyCode::Char('q') => self.pick = None,
                _ => {}
            }
            return Ok(None);
        }
        if self.confirm.is_some() {
            return Ok(match k.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let (_, pending) = self.confirm.take().unwrap();
                    self.marked.clear();
                    Some(Effect::InTerminal(match pending {
                        Pending::Local(list) => {
                            self.last_deleted = list.first().map(|b| b.name.clone()).unwrap_or_default();
                            list.iter().map(|b| branches::delete_args(&b.name)).collect()
                        }
                        Pending::Remote(list) => self.remote_deletes(list),
                    }))
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    self.confirm = None;
                    None
                }
                _ => None,
            });
        }
        if let Some(p) = &mut self.palette {
            match p.key(k) {
                Some(PaletteEvent::Close) => self.palette = None,
                Some(PaletteEvent::Run(cmd)) => {
                    self.palette = None;
                    return self.run(cmd);
                }
                None => {}
            }
            return Ok(None);
        }
        if self.searching {
            match k.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                    self.filter();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.filter();
                }
                KeyCode::Up => self.step(-1),
                KeyCode::Down => self.step(1),
                KeyCode::Char(c) if !ctrl => {
                    self.query.push(c);
                    self.selected = 0;
                    self.filter();
                }
                _ => {}
            }
            return Ok(None);
        }

        if self.detail_scroll.key(k, self.detail_area.height) {
            return Ok(None);
        }
        let cmd = match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(commands()));
                return Ok(None);
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(commands()));
                return Ok(None);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.step(1);
                return Ok(None);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.step(-1);
                return Ok(None);
            }
            KeyCode::PageDown => {
                self.step(15);
                return Ok(None);
            }
            KeyCode::PageUp => {
                self.step(-15);
                return Ok(None);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = 0;
                return Ok(None);
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.visible.len().saturating_sub(1);
                return Ok(None);
            }
            KeyCode::Enter => Cmd::Checkout,
            KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::SHIFT) => Cmd::DeleteRemote,
            KeyCode::Char('d') => Cmd::DeleteLocal,
            KeyCode::Char('D') => Cmd::DeleteRemote,
            KeyCode::Char(' ') => Cmd::Mark,
            KeyCode::Char('m') => Cmd::MarkMerged,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('o') => Cmd::OpenPr,
            KeyCode::Char('R') => Cmd::ReloadPrs,
            KeyCode::Char('r') if shift => Cmd::ReloadPrs,
            KeyCode::Char('r') => Cmd::Rename,
            KeyCode::Char('f') => Cmd::Fetch,
            KeyCode::Char('a') => Cmd::ToggleRemotes,
            KeyCode::Char('/') => Cmd::Search,
            KeyCode::Tab => Cmd::Details,
            KeyCode::Esc if !self.marked.is_empty() => Cmd::ClearMarks,
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.filter();
                return Ok(None);
            }
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Ok(None),
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Result<Option<Effect>> {
        match cmd {
            Cmd::Quit => return Ok(Some(Effect::Quit)),
            Cmd::Search => self.searching = true,
            Cmd::Details => self.details = !self.details,
            Cmd::ToggleRemotes => {
                self.remotes = !self.remotes;
                self.reload()?;
                let msg = if self.remotes { "Showing remote branches" } else { "Local branches only" };
                self.toast = Some(Toast::new(msg, theme::ACCENT));
            }
            Cmd::Copy => {
                if let Some(b) = self.current() {
                    let name = b.name.clone();
                    crate::clipboard::copy(&name);
                    self.toast = Some(Toast::new(format!("Copied {name}"), theme::ACCENT));
                }
            }
            Cmd::Rename => match self.current() {
                Some(b) if b.remote => {
                    self.toast = Some(Toast::new("Remote branches are renamed on the remote", theme::YELLOW));
                }
                Some(b) => {
                    let mut input = Input::new("Rename", &b.name, &b.name, "Rename");
                    if !b.copies.is_empty() {
                        input = input.note("The copy on the remote keeps its old name.");
                    }
                    self.rename = Some((input, b.name.clone()));
                }
                None => {}
            },
            Cmd::Fetch => self.fetch(),
            Cmd::OpenPr => {
                let msg = match (&self.prs, self.current().and_then(|b| self.pr(b))) {
                    (_, Some(pr)) => {
                        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                        let ok = Process::new(opener)
                            .arg(&pr.url)
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status()
                            .is_ok_and(|s| s.success());
                        if ok { format!("Opened #{}", pr.number) } else { pr.url.clone() }
                    }
                    (Prs::Loading(_), None) => "Still loading pull requests".into(),
                    (Prs::Off, None) => "Pull requests need a GitHub remote and gh".into(),
                    _ => "No pull request for this branch".into(),
                };
                self.toast = Some(Toast::new(msg, theme::ACCENT));
            }
            Cmd::ReloadPrs => {
                if matches!(self.prs, Prs::Loading(_)) {
                    return Ok(None);
                }
                self.load_prs();
                if matches!(self.prs, Prs::Loading(_)) {
                    self.toast = Some(Toast::new("Reloading pull requests…", theme::ACCENT));
                }
            }
            Cmd::Mark => {
                if let Some(b) = self.current() {
                    let name = b.name.clone();
                    if !self.marked.remove(&name) {
                        self.marked.insert(name);
                    }
                    self.step(1);
                }
            }
            Cmd::MarkMerged => {
                let picks: Vec<String> = self
                    .data
                    .list
                    .iter()
                    .filter(|b| {
                        !b.remote && (b.merged || b.gone() || self.merged_by_pr(b).is_some()) && !self.protected(b)
                    })
                    .map(|b| b.name.clone())
                    .collect();
                let msg = match picks.len() {
                    0 => "No merged or gone branches".to_string(),
                    n => format!("Selected {n} merged or gone branch{}", if n == 1 { "" } else { "es" }),
                };
                let msg = if matches!(self.prs, Prs::Loading(_)) {
                    format!("{msg} (pull requests still loading)")
                } else {
                    msg
                };
                self.marked.extend(picks);
                self.toast = Some(Toast::new(msg, theme::ACCENT));
            }
            Cmd::ClearMarks => self.marked.clear(),
            Cmd::Checkout => {
                let Some(b) = self.current().cloned() else {
                    return Ok(None);
                };
                // For origin/feat, `git switch feat` creates a local branch that tracks it.
                let target = if b.remote {
                    b.name.split_once('/').map_or(b.name.clone(), |(_, n)| n.to_string())
                } else {
                    b.name.clone()
                };
                if self.data.list.iter().any(|x| x.current && x.name == target) {
                    self.toast = Some(Toast::new(format!("Already on {target}"), theme::ACCENT));
                    return Ok(None);
                }
                if let Some(path) =
                    self.data.list.iter().find(|x| !x.remote && x.name == target).and_then(|x| x.worktree.as_ref())
                {
                    self.toast = Some(Toast::new(
                        format!("{target} is checked out in the worktree at {}", short_path(path)),
                        theme::YELLOW,
                    ));
                    return Ok(None);
                }
                let (ok, out) = branches::switch(&target);
                self.toast = Some(if ok {
                    Toast::new(format!("Switched to {target}"), theme::GREEN)
                } else {
                    Toast::new(first_lines(&out), theme::RED)
                });
                self.reload()?;
            }
            Cmd::DeleteLocal => self.ask_delete_local(),
            Cmd::DeleteRemote => self.ask_delete_remote(),
        }
        Ok(None)
    }
}
