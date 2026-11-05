//! Finding and running other programs (git, gh, fzf, pbcopy).

use std::path::Path;
use std::process::{Command, Stdio};

/// Is `bin` on the PATH?
pub fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}

/// What a finished program said.
pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// stderr then stdout, trimmed: where git explains itself.
    pub fn text(&self) -> String {
        format!("{}{}", self.stderr, self.stdout).trim().to_string()
    }
}

/// Runs `bin args…` in `dir` (or here), waits, and collects its output.
/// stdin is inherited so ssh and git can still ask on the terminal.
pub fn run(bin: &str, args: &[String], dir: Option<&Path>) -> std::io::Result<Output> {
    let mut cmd = Command::new(bin);
    cmd.args(args).stdin(Stdio::inherit());
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let out = cmd.output()?;
    Ok(Output {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}
