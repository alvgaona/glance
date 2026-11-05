//! Keys and palette commands of the stash screen.

use super::*;

impl App<'_> {
    pub(super) fn on_key(&mut self, k: KeyEvent) -> Flow<(), Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if let Some((input, _)) = &mut self.input {
            match input.key(k) {
                Some(InputEvent::Cancel) => self.input = None,
                Some(InputEvent::Done(text)) => {
                    let (_, ask) = self.input.take().unwrap();
                    return self.answer(ask, text);
                }
                None => {}
            }
            return Flow::Stay;
        }
        if ctrl && k.code == KeyCode::Char('c') {
            return Flow::Done(());
        }
        if self.confirm.is_some() {
            match k.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let (_, effect) = self.confirm.take().unwrap();
                    return Flow::Run(effect);
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => self.confirm = None,
                _ => {}
            }
            return Flow::Stay;
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
            return Flow::Stay;
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
            return Flow::Stay;
        }

        // Keys for both panes.
        match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(commands()));
                return Flow::Stay;
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(commands()));
                return Flow::Stay;
            }
            KeyCode::Tab | KeyCode::BackTab => return self.run(Cmd::Focus),
            _ => {}
        }

        if self.focus == Pane::Changes {
            if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
                return Flow::Done(());
            }
            if let Some(picked) = self.changes.key(k) {
                let input =
                    Input::new("Stash", &picked.describe(), "", "Stash").spaces().placeholder("message (optional)");
                self.input = Some((input, Ask::New(picked)));
            }
            return Flow::Stay;
        }

        if self.detail_scroll.key(k, self.detail_area.height) {
            return Flow::Stay;
        }
        let cmd = match k.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.step(1);
                return Flow::Stay;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.step(-1);
                return Flow::Stay;
            }
            KeyCode::PageDown => {
                self.step(15);
                return Flow::Stay;
            }
            KeyCode::PageUp => {
                self.step(-15);
                return Flow::Stay;
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = 0;
                return Flow::Stay;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.visible.len().saturating_sub(1);
                return Flow::Stay;
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
                return Flow::Stay;
            }
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Flow::Stay,
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Flow<(), Effect> {
        match cmd {
            Cmd::Quit => return Flow::Done(()),
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
                    return Flow::Run(Effect::Copy(s.oid.to_string()));
                }
            }
            Cmd::Apply | Cmd::Pop => {
                if let Some(stash) = self.current().cloned() {
                    return Flow::Run(Effect::Apply { stash, pop: matches!(cmd, Cmd::Pop) });
                }
            }
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
        Flow::Stay
    }
}
