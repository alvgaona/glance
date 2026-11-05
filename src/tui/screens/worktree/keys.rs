//! Keys and palette commands of the worktree screen.

use super::*;

impl App<'_> {
    pub(super) fn on_key(&mut self, k: KeyEvent) -> Flow<Option<PathBuf>, Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(input) = &mut self.input {
            match input.key(k) {
                Some(InputEvent::Cancel) => self.input = None,
                Some(InputEvent::Done(text)) => {
                    self.input = None;
                    let branch = text.trim();
                    if !branch.is_empty() {
                        return Flow::Run(Effect::Create(branch.to_string()));
                    }
                }
                None => {}
            }
            return Flow::Stay;
        }
        if ctrl && k.code == KeyCode::Char('c') {
            return Flow::Done(None);
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

        if self.detail_scroll.key(k, self.detail_area.height) {
            return Flow::Stay;
        }
        let cmd = match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(commands()));
                return Flow::Stay;
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(commands()));
                return Flow::Stay;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.step(1);
                return Flow::Stay;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.step(-1);
                return Flow::Stay;
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.step(isize::MIN / 2);
                return Flow::Stay;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.step(isize::MAX / 2);
                return Flow::Stay;
            }
            KeyCode::Enter => Cmd::Go,
            KeyCode::Char('n') => Cmd::New,
            KeyCode::Char('d') => Cmd::Remove,
            KeyCode::Char('p') => Cmd::Prune,
            KeyCode::Char('l') => Cmd::Lock,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('r') => Cmd::Reload,
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Flow::Stay,
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Flow<Option<PathBuf>, Effect> {
        match cmd {
            Cmd::Quit => return Flow::Done(None),
            Cmd::Reload => return Flow::Run(Effect::Reload),
            Cmd::Go => {
                if let Some(r) = self.current() {
                    if matches!(r.load, Load::None) && !r.wt.bare {
                        self.toast = Some(Toast::new("Its folder is gone: p prunes it", theme::YELLOW));
                    } else {
                        return Flow::Run(Effect::Go(r.wt.path.clone()));
                    }
                }
            }
            Cmd::Copy => {
                if let Some(r) = self.current() {
                    return Flow::Run(Effect::Copy(r.wt.path.clone()));
                }
            }
            Cmd::New => {
                let main = self.main().unwrap_or_default();
                let name = main.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let head = &self.head;
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
                    return Flow::Run(Effect::Lock { path: r.wt.path.clone(), on: r.wt.locked.is_none() });
                }
            }
        }
        Flow::Stay
    }
}
