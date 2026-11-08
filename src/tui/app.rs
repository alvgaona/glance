//! The event loop every screen runs in: expire the toast, do background work, draw,
//! wait for a key or a click, repeat until the screen is done.

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyEvent, KeyEventKind, MouseEvent};
use ratatui::Frame;

use super::kit::Toast;
use super::term::{enter, leave};

/// What a screen wants after a key or an effect.
pub(crate) enum Flow<T, E> {
    Stay,
    /// Close the screen with this answer.
    Done(T),
    /// Do this side effect (in [`Screen::apply`]) before drawing again.
    Run(E),
}

impl<T, E> From<Option<T>> for Flow<T, E> {
    fn from(o: Option<T>) -> Self {
        o.map_or(Flow::Stay, Flow::Done)
    }
}

/// A full screen. Keys change its state only; everything that touches the world (git,
/// other programs, the clipboard) comes back as an effect, which [`Screen::apply`] does.
pub(crate) trait Screen {
    type Out;
    /// What this screen does outside its own state.
    type Effect;

    /// How long to wait for input before drawing again (background work shows up then).
    const WAKE: Duration = Duration::from_millis(200);

    fn draw(&mut self, f: &mut Frame);
    fn key(&mut self, k: KeyEvent) -> Flow<Self::Out, Self::Effect>;
    fn mouse(&mut self, _m: MouseEvent) {}
    /// The one place a screen touches the world. May ask for another effect.
    fn apply(&mut self, e: Self::Effect) -> Result<Flow<Self::Out, Self::Effect>>;
    /// True when `e` needs the normal terminal: git may prompt, or an editor opens.
    fn in_terminal(_e: &Self::Effect) -> bool {
        false
    }
    /// Background results and lazy loads, before each draw.
    fn tick(&mut self) -> Result<()> {
        Ok(())
    }
    fn toast(&mut self) -> &mut Option<Toast>;
    /// The repository to reload on when it changes outside glance, and whether edits in
    /// the working tree count too. `None`: never.
    fn watched(&self) -> Option<(&gix::Repository, bool)> {
        None
    }
    /// Reads the repository again after it changed outside glance.
    fn refresh(&mut self) -> Result<()> {
        Ok(())
    }
    /// True while a text field or search has the keys: only then is a paste typed in.
    fn typing(&self) -> bool {
        false
    }
}

pub(crate) fn run<S: Screen>(screen: &mut S) -> Result<S::Out> {
    let mut terminal = enter();
    let watch = screen.watched().map(|(repo, edits)| super::watch::start(repo, edits));
    // The last stamp, and when this screen last wrote: a stamp begun before that may
    // have caught the write halfway, so it only sets the next baseline.
    let (mut seen, mut wrote) = (None, Instant::now());
    let result = (|| -> Result<S::Out> {
        loop {
            if screen.toast().as_ref().is_some_and(Toast::expired) {
                *screen.toast() = None;
            }
            for (at, stamp) in watch.iter().flat_map(|rx| rx.try_iter()) {
                if at < wrote {
                    continue;
                }
                if seen.is_some_and(|s| s != stamp) {
                    screen.refresh()?;
                    (seen, wrote) = (None, Instant::now());
                } else {
                    seen = Some(stamp);
                }
            }
            // Keys already waiting go first: holding j skips the commits in between
            // instead of loading each one.
            if !event::poll(Duration::ZERO)? {
                screen.tick()?;
                terminal.draw(|f| screen.draw(f))?;
            }
            if !event::poll(S::WAKE)? {
                continue;
            }
            let mut flow = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => screen.key(k),
                Event::Mouse(m) => {
                    screen.mouse(m);
                    Flow::Stay
                }
                Event::Paste(text) if screen.typing() => {
                    paste(screen, &text);
                    Flow::Stay
                }
                _ => Flow::Stay,
            };
            // Effects may ask for more effects (a write, then a reload).
            loop {
                flow = match flow {
                    Flow::Stay => break,
                    Flow::Done(out) => return Ok(out),
                    Flow::Run(e) if S::in_terminal(&e) => {
                        leave();
                        let next = screen.apply(e);
                        // Back to the full screen before any error goes up.
                        terminal = enter();
                        next?
                    }
                    Flow::Run(e) => screen.apply(e)?,
                };
                (seen, wrote) = (None, Instant::now());
            }
        }
    })();
    leave();
    result
}

/// Types `text` in as plain characters. A line break becomes a space, so a paste can
/// never press Enter (which would submit, and the rest would run as commands).
pub(crate) fn paste<S: Screen>(screen: &mut S, text: &str) {
    let chars = text.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).filter(|c| !c.is_control());
    for c in chars {
        let k = KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE);
        // Typing never leaves the field, so the flow is always Stay.
        let _ = screen.key(k);
    }
}

/// A plain key press.
#[cfg(test)]
pub(crate) fn press(code: crossterm::event::KeyCode) -> KeyEvent {
    KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

#[cfg(test)]
pub(crate) fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::CONTROL)
}

/// Types `text` into whatever has the keys; every key must leave the screen open.
#[cfg(test)]
pub(crate) fn type_in<S: Screen>(s: &mut S, text: &str) {
    for c in text.chars() {
        assert!(matches!(s.key(press(crossterm::event::KeyCode::Char(c))), Flow::Stay));
    }
}
