//! The event loop every screen runs in: expire the toast, do background work, draw,
//! wait for a key or a click, repeat until the screen is done.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEvent, KeyEventKind, MouseEvent};
use ratatui::Frame;

use super::term::{enter, leave};
use super::widgets::Toast;

/// What a screen wants after a key.
pub(crate) enum Flow<T> {
    Stay,
    /// Close the screen with this answer.
    Done(T),
    /// Give the terminal back for a moment (for commands that may prompt), then
    /// call [`Screen::outside`] and come back.
    Outside,
}

impl<T> From<Option<T>> for Flow<T> {
    fn from(o: Option<T>) -> Self {
        o.map_or(Flow::Stay, Flow::Done)
    }
}

pub(crate) trait Screen {
    type Out;

    /// How long to wait for input before drawing again (background work shows up then).
    const WAKE: Duration = Duration::from_millis(200);

    fn draw(&mut self, f: &mut Frame);
    fn key(&mut self, k: KeyEvent) -> Result<Flow<Self::Out>>;
    fn mouse(&mut self, _m: MouseEvent) {}
    /// Background results to pick up before each draw.
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
            let flow = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => screen.key(k)?,
                Event::Mouse(m) => {
                    screen.mouse(m);
                    Flow::Stay
                }
                _ => Flow::Stay,
            };
            match flow {
                Flow::Stay => {}
                Flow::Done(out) => return Ok(out),
                Flow::Outside => {
                    leave();
                    let done = screen.outside();
                    terminal = enter();
                    done?;
                }
            }
        }
    })();
    leave();
    result
}
