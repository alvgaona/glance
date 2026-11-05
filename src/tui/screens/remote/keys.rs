//! Keys and palette commands of the remote screen.

use super::*;

impl App {
    pub(super) fn on_key(&mut self, k: KeyEvent) -> Flow<(), Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if let Some((input, _)) = &mut self.input {
            match input.key(k) {
                Some(InputEvent::Cancel) => self.input = None,
                Some(InputEvent::Done(text)) => {
                    let (_, typing) = self.input.take().unwrap();
                    return self.typed(typing, text);
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
            KeyCode::Home | KeyCode::Char('g') => {
                self.step(isize::MIN / 2);
                return Flow::Stay;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.step(isize::MAX / 2);
                return Flow::Stay;
            }
            KeyCode::Char('f') | KeyCode::Enter => Cmd::Fetch,
            KeyCode::Char('F') => Cmd::FetchAll,
            KeyCode::Char('a') => Cmd::Add,
            KeyCode::Char('r') => Cmd::Rename,
            KeyCode::Char('u') => Cmd::SetUrl,
            KeyCode::Char('U') => Cmd::SetPushUrl,
            KeyCode::Char('d') => Cmd::Remove,
            KeyCode::Char('h') => Cmd::SetHead,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('o') => Cmd::Open,
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Flow::Stay,
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Flow<(), Effect> {
        // Everything but these works on the remote under the cursor.
        let name = self.current().map(|r| r.name.clone());
        match (cmd, name) {
            (Cmd::Quit, _) => return Flow::Done(()),
            (Cmd::Reload, _) => return Flow::Run(Effect::Reload),
            (Cmd::FetchAll, _) => return Flow::Run(Effect::Fetch(None)),
            (Cmd::Add, _) => {
                let input = Input::new("Add a remote to", &self.repo_name, "", "Next")
                    .placeholder("URL, e.g. git@github.com:me/repo.git")
                    .note("A URL or a path to another repository.\nYou name it next.");
                self.input = Some((input, Typing::AddUrl));
            }
            (_, None) => self.toast = Some(Toast::new("No remotes yet: a adds one", theme::ACCENT)),
            (Cmd::Fetch, Some(n)) => return Flow::Run(Effect::Fetch(Some(n))),
            (Cmd::Rename, Some(n)) => {
                let r = self.current().unwrap();
                let mut note = "Its remote branches move to <new>/….".to_string();
                match r.tracking.len() {
                    0 => {}
                    1 => note += &format!("\n{} keeps tracking it.", r.tracking[0]),
                    t => note += &format!("\nThe {t} local branches tracking it follow."),
                }
                let input = Input::new("Rename", &n, &n, "Rename").note(note);
                self.input = Some((input, Typing::Rename { old: n }));
            }
            (Cmd::SetUrl, Some(n)) => {
                let was = self.current().and_then(|r| r.url.clone()).unwrap_or_default();
                let input = Input::new("Fetch URL of", &n, &was, "Save").placeholder("URL");
                self.input = Some((input, Typing::Url { name: n }));
            }
            (Cmd::SetPushUrl, Some(n)) => {
                let r = self.current().unwrap();
                let was = r.push_url.clone().or_else(|| r.url.clone()).unwrap_or_default();
                let input = Input::new("Push URL of", &n, &was, "Save")
                    .placeholder("URL")
                    .note("Leave it empty to push where it fetches from.");
                self.input = Some((input, Typing::PushUrl { name: n }));
            }
            (Cmd::Remove, Some(_)) => self.ask_remove(),
            (Cmd::SetHead, Some(n)) => return Flow::Run(Effect::SetHead(n)),
            (Cmd::Copy, Some(n)) => match self.current().and_then(|r| r.url.clone()) {
                Some(u) => return Flow::Run(Effect::Copy(u)),
                None => self.toast = Some(Toast::new(format!("{n} has no URL"), theme::YELLOW)),
            },
            (Cmd::Open, Some(n)) => match self.current().and_then(|r| r.url.as_deref()).and_then(url::web) {
                Some(page) => return Flow::Run(Effect::Open(page)),
                None => self.toast = Some(Toast::new(format!("{n} isn't on a web host"), theme::YELLOW)),
            },
        }
        Flow::Stay
    }
}
