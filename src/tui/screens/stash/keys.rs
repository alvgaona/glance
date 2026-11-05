//! Keys and palette commands of the stash screen.

use super::*;

impl App<'_> {
    /// Returns true to quit.
    pub(super) fn on_key(&mut self, k: KeyEvent) -> Result<bool> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if let Some((input, _)) = &mut self.input {
            match input.key(k) {
                Some(InputEvent::Cancel) => self.input = None,
                Some(InputEvent::Done(text)) => {
                    let (_, ask) = self.input.take().unwrap();
                    self.answer(ask, text)?;
                }
                None => {}
            }
            return Ok(false);
        }
        if ctrl && k.code == KeyCode::Char('c') {
            return Ok(true);
        }
        if self.confirm.is_some() {
            match k.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let (_, list) = self.confirm.take().unwrap();
                    self.drop_stashes(list)?;
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
            return Ok(false);
        }

        // Keys for both panes.
        match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(commands()));
                return Ok(false);
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(commands()));
                return Ok(false);
            }
            KeyCode::Tab | KeyCode::BackTab => return self.run(Cmd::Focus),
            _ => {}
        }

        if self.focus == Pane::Changes {
            if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
                return Ok(true);
            }
            if let Some(picked) = self.changes.key(k) {
                let input =
                    Input::new("Stash", &picked.describe(), "", "Stash").spaces().placeholder("message (optional)");
                self.input = Some((input, Ask::New(picked)));
            }
            return Ok(false);
        }

        if self.detail_scroll.key(k, self.detail_area.height) {
            return Ok(false);
        }
        let cmd = match k.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.step(1);
                return Ok(false);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.step(-1);
                return Ok(false);
            }
            KeyCode::PageDown => {
                self.step(15);
                return Ok(false);
            }
            KeyCode::PageUp => {
                self.step(-15);
                return Ok(false);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = 0;
                return Ok(false);
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.visible.len().saturating_sub(1);
                return Ok(false);
            }
            KeyCode::Enter | KeyCode::Char('a') => Cmd::Apply,
            KeyCode::Char('p') => Cmd::Pop,
            KeyCode::Char('d') => Cmd::Drop,
            KeyCode::Char('b') => Cmd::Branch,
            KeyCode::Char('n') => Cmd::New(Mode::Everything),
            KeyCode::Char(' ') => Cmd::Mark,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('/') => Cmd::Search,
            KeyCode::Esc if !self.marked.is_empty() => Cmd::ClearMarks,
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.filter();
                return Ok(false);
            }
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Ok(false),
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Result<bool> {
        match cmd {
            Cmd::Quit => return Ok(true),
            Cmd::Search => {
                self.focus = Pane::Stashes;
                self.searching = true;
            }
            Cmd::Focus => {
                self.focus = match self.focus {
                    Pane::Changes => Pane::Stashes,
                    Pane::Stashes => Pane::Changes,
                }
            }
            Cmd::ClearMarks => self.marked.clear(),
            Cmd::Mark => {
                if let Some(s) = self.current() {
                    let id = s.oid;
                    if !self.marked.remove(&id) {
                        self.marked.insert(id);
                    }
                    self.step(1);
                }
            }
            Cmd::Copy => {
                if let Some(s) = self.current() {
                    let id = s.oid.to_string();
                    crate::clipboard::copy(&id);
                    self.toast = Some(Toast::new(format!("Copied {}", &id[..7.min(id.len())]), theme::ACCENT));
                }
            }
            Cmd::Apply => self.apply(false)?,
            Cmd::Pop => self.apply(true)?,
            Cmd::Drop => self.ask_drop(),
            Cmd::Branch => {
                if let Some(s) = self.current().cloned() {
                    let input = Input::new("New branch from", &s.name(), &suggest_branch(&s), "Create branch").note(
                        format!("Checks out a new branch at {}, applies the stash there\nand drops it.", s.base_id),
                    );
                    self.input = Some((input, Ask::Branch(s)));
                }
            }
            Cmd::New(mode) => {
                self.focus = Pane::Changes;
                self.changes.set_mode(mode);
            }
            Cmd::PickUntracked => {
                self.focus = Pane::Changes;
                self.changes.toggle_untracked();
            }
            Cmd::PickAll => {
                self.focus = Pane::Changes;
                self.changes.toggle_all();
            }
        }
        Ok(false)
    }
}
