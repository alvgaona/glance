//! Keys and palette commands of the staging screen.

use super::*;

impl App {
    /// `true` to quit.
    pub(super) fn on_key(&mut self, k: KeyEvent) -> Result<bool> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Ok(true);
        }
        if self.commit.is_some() {
            self.commit_key(k)?;
            return Ok(false);
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
            return Ok(false);
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
            return Ok(false);
        }

        if self.detail_scroll.key(k, self.detail_area.height) {
            return Ok(false);
        }
        if self.focus == Focus::Hunks {
            match k.code {
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('n') => self.step_hunk(1),
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('N') => self.step_hunk(-1),
                KeyCode::Home | KeyCode::Char('g') => self.step_hunk(isize::MIN / 2),
                KeyCode::End | KeyCode::Char('G') => self.step_hunk(isize::MAX / 2),
                KeyCode::Char(' ') | KeyCode::Enter => self.toggle_hunk()?,
                KeyCode::Char('d') => self.ask_discard_hunk(),
                KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                    self.focus = Focus::Files
                }
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Char('p') if ctrl => self.palette = Some(Palette::new(commands())),
                KeyCode::Char(':') => self.palette = Some(Palette::new(commands())),
                KeyCode::Char('y') => return self.run(Cmd::Copy),
                KeyCode::Char('c') => self.open_commit(false),
                KeyCode::Char('C') => self.open_commit(true),
                _ => {}
            }
            return Ok(false);
        }
        let cmd = match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(commands()));
                return Ok(false);
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(commands()));
                return Ok(false);
            }
            KeyCode::Char('r') if ctrl => Cmd::Reload,
            KeyCode::Down | KeyCode::Char('j') => {
                self.step(1);
                return Ok(false);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.step(-1);
                return Ok(false);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.step(isize::MIN / 2);
                return Ok(false);
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.step(isize::MAX / 2);
                return Ok(false);
            }
            KeyCode::Char(' ') => Cmd::Toggle,
            KeyCode::Enter | KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => Cmd::Hunks,
            KeyCode::Char('a') => Cmd::StageAll,
            KeyCode::Char('u') => Cmd::UnstageAll,
            KeyCode::Char('d') => Cmd::Discard,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('c') => Cmd::Commit,
            KeyCode::Char('C') => Cmd::Amend,
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Ok(false),
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Result<bool> {
        match cmd {
            Cmd::Quit => return Ok(true),
            Cmd::Reload => self.reload()?,
            Cmd::Toggle => self.toggle()?,
            Cmd::Hunks => self.enter_hunks(),
            Cmd::Commit => self.open_commit(false),
            Cmd::Amend => self.open_commit(true),
            Cmd::Discard => self.ask_discard(),
            Cmd::StageAll => {
                let n = self.entries.iter().filter(|e| e.group != Group::Staged).count();
                if n == 0 {
                    self.toast = Some(Toast::new("Everything is staged already", theme::ACCENT));
                } else {
                    let r = stage::add_all(&self.repo);
                    self.done(r, "Staged everything".into())?;
                }
            }
            Cmd::UnstageAll => {
                if !self.entries.iter().any(|e| e.group == Group::Staged) {
                    self.toast = Some(Toast::new("Nothing is staged", theme::ACCENT));
                } else {
                    let r = stage::unstage_all(&self.repo);
                    self.done(r, "Unstaged everything".into())?;
                }
            }
            Cmd::Copy => {
                if let Some(e) = self.current() {
                    let path = e.path.clone();
                    crate::clipboard::copy(&path);
                    self.toast = Some(Toast::new(format!("Copied {path}"), theme::ACCENT));
                }
            }
        }
        Ok(false)
    }
}
