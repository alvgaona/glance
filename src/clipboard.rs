//! The system clipboard.

use std::io::Write;
use std::process::{Command, Stdio};

/// Puts `text` on the clipboard (pbcopy on macOS). Silently does nothing without one.
pub fn copy(text: &str) {
    if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}
