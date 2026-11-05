//! Pieces every screen is drawn with: text fitting, toast, dialogs, palette, footer,
//! list scrolling and the details panel's scroll.

mod dialog;
pub mod footer;
mod input;
mod layout;
pub mod list;
mod palette;
mod scroll;
mod text;
mod toast;

pub use dialog::Confirm;
pub use input::{Input, InputEvent};
pub use layout::{centered, panel};
pub use palette::{Command, Palette, PaletteEvent};
pub use scroll::Scroll;
pub use text::{pill, truncate, wrap};
pub use toast::Toast;
