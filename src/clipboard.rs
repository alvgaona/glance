//! The system clipboard.

/// Puts `text` on the clipboard (pbcopy on macOS). Silently does nothing without one.
pub fn copy(text: &str) {
    let _ = crate::process::run_with_input("pbcopy", &[], None, text.as_bytes(), &[]);
}
