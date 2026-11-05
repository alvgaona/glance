//! Keys and palette commands of the staging screen.

use super::*;

impl App {
    pub(super) fn on_key(&mut self, k: KeyEvent) -> Flow<(), Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Flow::Done(());
        }
        if self.commit.is_some() {
            return self.commit_key(k).map_or(Flow::Stay, Flow::Run);
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

        if self.focus == Focus::Hunks && self.pick().is_some() {
            let shift = k.modifiers.contains(KeyModifiers::SHIFT);
            match k.code {
                KeyCode::Down | KeyCode::Char('j') => self.step_pick(true, shift),
                KeyCode::Up | KeyCode::Char('k') => self.step_pick(false, shift),
                KeyCode::Char('J') => self.step_pick(true, true),
                KeyCode::Char('K') => self.step_pick(false, true),
                KeyCode::Char(' ') | KeyCode::Enter => return self.toggle_lines().map_or(Flow::Stay, Flow::Run),
                KeyCode::Char('d') => self.ask_discard_lines(),
                KeyCode::Char('v') | KeyCode::Esc => self.pick = None,
                KeyCode::Char('q') => return Flow::Done(()),
                // Anything else leaves the pick behind and works as usual.
                _ => {
                    self.pick = None;
                    return self.on_key(k);
                }
            }
            return Flow::Stay;
        }
        // After the pick: J/K there grow it instead of scrolling.
        if self.detail_scroll.key(k, self.detail_area.height) {
            return Flow::Stay;
        }
        if self.focus == Focus::Hunks {
            match k.code {
                KeyCode::Char('v') => self.toggle_pick(),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('n') => self.step_hunk(1),
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('N') => self.step_hunk(-1),
                KeyCode::Home | KeyCode::Char('g') => self.step_hunk(isize::MIN / 2),
                KeyCode::End | KeyCode::Char('G') => self.step_hunk(isize::MAX / 2),
                KeyCode::Char(' ') | KeyCode::Enter => return self.toggle_hunk().map_or(Flow::Stay, Flow::Run),
                KeyCode::Char('d') => self.ask_discard_hunk(),
                KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                    self.focus = Focus::Files
                }
                KeyCode::Char('q') => return Flow::Done(()),
                KeyCode::Char('p') if ctrl => self.palette = Some(Palette::new(commands())),
                KeyCode::Char(':') => self.palette = Some(Palette::new(commands())),
                KeyCode::Char('y') => return self.run(Cmd::Copy),
                KeyCode::Char('e') => return self.run(Cmd::Edit),
                KeyCode::Char('c') => return self.run(Cmd::Commit),
                KeyCode::Char('C') => return self.run(Cmd::Amend),
                _ => {}
            }
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
            KeyCode::Char('r') if ctrl => Cmd::Reload,
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
            KeyCode::Char(' ') => Cmd::Toggle,
            KeyCode::Enter | KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => Cmd::Hunks,
            KeyCode::Char('a') => Cmd::StageAll,
            KeyCode::Char('u') => Cmd::UnstageAll,
            KeyCode::Char('d') => Cmd::Discard,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('e') => Cmd::Edit,
            KeyCode::Char('c') => Cmd::Commit,
            KeyCode::Char('C') => Cmd::Amend,
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Flow::Stay,
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Flow<(), Effect> {
        let effect = match cmd {
            Cmd::Quit => return Flow::Done(()),
            Cmd::Reload => Some(Effect::Reload),
            Cmd::Toggle => self.toggle(),
            Cmd::Hunks => {
                self.enter_hunks();
                None
            }
            Cmd::Commit => self.open_commit(false),
            Cmd::Amend => self.open_commit(true),
            Cmd::Discard => {
                self.ask_discard();
                None
            }
            Cmd::Edit => self.ask_edit(),
            Cmd::StageAll if !self.entries.iter().any(|e| e.group != Group::Staged) => {
                self.toast = Some(Toast::new("Everything is staged already", theme::ACCENT));
                None
            }
            Cmd::StageAll => Some(Effect::StageAll),
            Cmd::UnstageAll if !self.entries.iter().any(|e| e.group == Group::Staged) => {
                self.toast = Some(Toast::new("Nothing is staged", theme::ACCENT));
                None
            }
            Cmd::UnstageAll => Some(Effect::UnstageAll),
            Cmd::Copy => self.current().map(|e| Effect::Copy(e.path.clone())),
        };
        effect.map_or(Flow::Stay, Flow::Run)
    }
}
