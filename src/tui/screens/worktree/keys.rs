//! Keys and palette commands of the worktree screen.

use super::*;

impl App<'_> {
    /// Returns `Some` to quit, with the folder picked (if any).
    pub(super) fn on_key(&mut self, k: KeyEvent) -> Result<Option<Option<PathBuf>>> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(input) = &mut self.input {
            match input.key(k) {
                Some(InputEvent::Cancel) => self.input = None,
                Some(InputEvent::Done(text)) => {
                    self.input = None;
                    self.create(text.trim())?;
                }
                None => {}
            }
            return Ok(None);
        }
        if ctrl && k.code == KeyCode::Char('c') {
            return Ok(Some(None));
        }
        if self.confirm.is_some() {
            match k.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let (_, ask) = self.confirm.take().unwrap();
                    self.answer(ask)?;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => self.confirm = None,
                _ => {}
            }
            return Ok(None);
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
            KeyCode::Home | KeyCode::Char('g') => {
                self.step(isize::MIN / 2);
                return Ok(None);
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.step(isize::MAX / 2);
                return Ok(None);
            }
            KeyCode::Enter => Cmd::Go,
            KeyCode::Char('n') => Cmd::New,
            KeyCode::Char('d') => Cmd::Remove,
            KeyCode::Char('p') => Cmd::Prune,
            KeyCode::Char('l') => Cmd::Lock,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('r') => Cmd::Reload,
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Ok(None),
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Result<Option<Option<PathBuf>>> {
        match cmd {
            Cmd::Quit => return Ok(Some(None)),
            Cmd::Reload => self.reload(None)?,
            Cmd::Go => {
                if let Some(r) = self.current() {
                    if matches!(r.load, Load::None) && !r.wt.bare {
                        self.toast = Some(Toast::new("Its folder is gone: p prunes it", theme::YELLOW));
                    } else {
                        let path = r.wt.path.clone();
                        // Printed on the way out; in a plain terminal it's copied too.
                        if !self.captured {
                            crate::clipboard::copy(&path.display().to_string());
                        }
                        return Ok(Some(Some(path)));
                    }
                }
            }
            Cmd::Copy => {
                if let Some(r) = self.current() {
                    crate::clipboard::copy(&r.wt.path.display().to_string());
                    self.toast = Some(Toast::new(format!("Copied {}", tilde(&r.wt.path)), theme::ACCENT));
                }
            }
            Cmd::New => {
                let main = self.main().unwrap_or_default();
                let name = main.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let head = self.repo.head_id().map(|h| h.shorten_or_id().to_string()).unwrap_or_default();
                self.input = Some(
                    Input::new("New worktree", &self.repo_name, "", "Create")
                        .placeholder("branch name")
                        .note(format!(
                            "Goes next to {name}, as {name}-<branch>. An existing branch (local or\non a remote) is checked out there; a new one starts from HEAD ({head})."
                        )),
                );
            }
            Cmd::Remove => self.ask_remove(),
            Cmd::Prune => self.ask_prune(),
            Cmd::Lock => {
                if let Some(r) = self.current() {
                    let on = r.wt.locked.is_none();
                    let done = if on { "Locked" } else { "Unlocked" };
                    let (ok, out) = worktree::lock(&r.wt.path, on);
                    let name = self.name(&r.wt.path);
                    self.toast = Some(if ok {
                        Toast::new(format!("{done} {name}"), theme::GREEN)
                    } else {
                        Toast::new(first_lines(&out), theme::RED)
                    });
                    self.reload(None)?;
                }
            }
        }
        Ok(None)
    }
}
