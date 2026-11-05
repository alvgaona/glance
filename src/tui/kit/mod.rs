//! Pieces every screen is drawn with: text fitting, toast, dialogs, palette, footer.

mod dialog;
pub mod footer;
mod input;
mod layout;
mod palette;
mod text;
mod toast;

pub use dialog::Confirm;
pub use input::{Input, InputEvent};
pub use layout::{centered, panel};
pub use palette::{Command, Palette, PaletteEvent};
pub use text::{pill, truncate, wrap};
pub use toast::Toast;
