//! One module per subcommand: read what it needs from `git`, then print it or open its
//! full screen. The only layer that knows both `print` and `tui`.

pub mod add;
pub mod branches;
pub mod log;
pub mod rebase;
pub mod remote;
pub mod repos;
pub mod stash;
pub mod status;
pub mod worktree;

use std::io::IsTerminal;

/// Full screen when output goes to a terminal (and gg was built with screens).
/// Piped output gets the printed version.
fn screen_for_stdout() -> bool {
    cfg!(feature = "tui") && std::io::stdout().is_terminal()
}

/// Full screen whenever someone is at the keyboard, even with stdout captured:
/// for commands whose answer is printed for the shell, like `cd "$(gg wt)"`.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
fn screen_for_keyboard() -> bool {
    cfg!(feature = "tui") && std::io::stdin().is_terminal()
}
