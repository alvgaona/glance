//! delta draws the diffs in `glance diff` when it's installed.

use crate::process::{run, run_with_input, which};

pub fn available() -> bool {
    which("delta")
}

/// What the diff screen controls in delta's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Look {
    pub split: bool,
    pub numbers: bool,
}

/// `patch` drawn by delta at `width` columns: text with colour codes. The user's delta
/// settings (theme, styles) still apply; only what the screen controls is overridden.
pub fn render(patch: &str, width: u16, look: Look) -> Result<String, String> {
    let set = |key: &str, value: &str| format!("'delta.{key}'='{value}'");
    let params = [
        // Kept: a `git -c` from the user still counts.
        std::env::var("GIT_CONFIG_PARAMETERS").unwrap_or_default(),
        set("side-by-side", &look.split.to_string()),
        set("line-numbers", &look.numbers.to_string()),
        // The pane's title already names the file.
        set("file-style", "omit"),
        set("file-decoration-style", "none"),
        set("hyperlinks", "false"),
    ]
    .join(" ");
    let args: Vec<String> = ["--paging=never", "--line-fill-method=spaces", &format!("--width={width}")]
        .into_iter()
        .map(String::from)
        .collect();
    let env = [("GIT_CONFIG_PARAMETERS", params.trim())];
    match run_with_input("delta", &args, None, patch.as_bytes(), &env) {
        Ok(out) if out.ok() => Ok(out.stdout),
        Ok(out) => Err(out.stderr.trim().to_string()),
        Err(e) => Err(format!("could not run delta: {e}")),
    }
}

/// A delta switch from git config (`delta.side-by-side`), when it's set.
pub fn setting(key: &str) -> Option<bool> {
    let args = ["config", "--type=bool", "--get", &format!("delta.{key}")].map(String::from);
    let out = run("git", &args, None).ok()?;
    match out.stdout.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}
