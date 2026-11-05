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

/// Runs `bin args…` in `dir` with `input` on its stdin (a patch for `git apply`), waits,
/// and collects its output.
pub fn run_with_input(bin: &str, args: &[String], dir: Option<&Path>, input: &[u8]) -> std::io::Result<Output> {
    use std::io::Write;
    let mut cmd = Command::new(bin);
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let mut child = cmd.spawn()?;
    // Written from another thread so a chatty program can't block us both.
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let out = child.wait_with_output()?;
    // A program that stops reading early says why on stderr; that beats "broken pipe".
    let _ = writer.join();
    Ok(Output {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
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
