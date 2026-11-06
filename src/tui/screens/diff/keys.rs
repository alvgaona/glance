//! Keys and palette commands of the diff screen.

use super::*;

impl App {
    pub(super) fn on_key(&mut self, k: KeyEvent) -> Flow<(), Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Flow::Done(());
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
            KeyCode::Char('r') if ctrl => Cmd::Reload,
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
            KeyCode::Char('1') => Cmd::Compare(Target::Unstaged),
            KeyCode::Char('2') => Cmd::Compare(Target::Staged),
            KeyCode::Char('3') => Cmd::Compare(Target::Head),
            KeyCode::Char('s') => Cmd::Split,
            KeyCode::Char('l') => Cmd::Numbers,
            KeyCode::Char('w') => Cmd::Whitespace,
            KeyCode::Char('+' | '=') => Cmd::More,
            KeyCode::Char('-') => Cmd::Less,
            KeyCode::Char('f') => Cmd::Full,
            KeyCode::Char('n') => Cmd::Hunk(true),
            KeyCode::Char('N') => Cmd::Hunk(false),
            KeyCode::Char('z') => Cmd::Files,
            KeyCode::Char('i') => Cmd::Side,
            KeyCode::Char('e') => Cmd::Edit,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('/') => Cmd::Search,
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
        let path = self.current().map(|f| f.path.clone());
        match cmd {
            Cmd::Quit => return Flow::Done(()),
            Cmd::Compare(t) if t == self.target => {}
            Cmd::Compare(t) => {
                self.target = t;
                self.selected = 0;
                return Flow::Run(Effect::Load);
            }
            // Fixed from here on, whatever the width.
            Cmd::Split => self.split = Some(!self.split_at(self.width)),
            Cmd::Numbers => self.numbers = !self.numbers,
            Cmd::Whitespace => {
                self.cut.whitespace = !self.cut.whitespace;
                // Files with only whitespace changes come and go.
                return Flow::Run(Effect::Load);
            }
            Cmd::More | Cmd::Less => {
                let i = CONTEXT.iter().position(|&c| c >= self.cut.context).unwrap_or(CONTEXT.len() - 1);
                let i = if matches!(cmd, Cmd::More) { (i + 1).min(CONTEXT.len() - 1) } else { i.saturating_sub(1) };
                self.cut = Cut { context: CONTEXT[i], full: false, ..self.cut };
            }
            Cmd::Full => self.cut.full = !self.cut.full,
            Cmd::Hunk(down) => self.jump(down),
            Cmd::Files => self.hide_files = !self.hide_files,
            Cmd::Side => self.side = !self.side,
            Cmd::Edit => return path.map_or(Flow::Stay, |p| Flow::Run(Effect::Edit(p, self.line_here()))),
            Cmd::Copy => return path.map_or(Flow::Stay, |p| Flow::Run(Effect::Copy(p))),
            Cmd::Search => {
                self.hide_files = false;
                self.searching = true;
            }
            Cmd::Reload => return Flow::Run(Effect::Load),
        }
        Flow::Stay
    }
}
