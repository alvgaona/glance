//! The event loop every screen runs in: expire the toast, do background work, draw,
//! wait for a key or a click, repeat until the screen is done.

use std::time::Duration;

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
    /// Give the terminal back for a moment (for commands that may prompt), then
    /// call [`Screen::outside`] and come back.
    Outside,
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
    fn key(&mut self, k: KeyEvent) -> Result<Flow<Self::Out, Self::Effect>>;
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
    /// Runs on the normal screen after [`Flow::Outside`].
    fn outside(&mut self) -> Result<()> {
        Ok(())
    }
}

pub(crate) fn run<S: Screen>(screen: &mut S) -> Result<S::Out> {
    let mut terminal = enter();
    let result = (|| -> Result<S::Out> {
        loop {
            if screen.toast().as_ref().is_some_and(Toast::expired) {
                *screen.toast() = None;
            }
            screen.tick()?;
            terminal.draw(|f| screen.draw(f))?;
            if !event::poll(S::WAKE)? {
                continue;
            }
            let mut flow = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => screen.key(k)?,
                Event::Mouse(m) => {
                    screen.mouse(m);
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
                    Flow::Outside => {
                        leave();
                        let done = screen.outside();
                        terminal = enter();
                        done?;
                        Flow::Stay
                    }
                };
            }
        }
    })();
    leave();
    result
}
