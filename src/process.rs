//! Finding and running other programs (git, gh, pbcopy). The only place that
//! starts a process: clippy.toml bans `Command::new` everywhere else.
#![allow(clippy::disallowed_methods)]

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
    /// Exit code; `None` when a signal stopped it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// stderr then stdout, trimmed: where git explains itself.
    pub fn text(&self) -> String {
        format!("{}{}", self.stderr, self.stdout).trim().to_string()
    }
}

/// Runs `bin args…` in `dir` with `input` on its stdin (a patch for `git apply`; empty for
/// a job that must not read the terminal), with `env` set, waits, and collects its output.
pub fn run_with_input(
    bin: &str,
    args: &[String],
    dir: Option<&Path>,
    input: &[u8],
    env: &[(&str, &str)],
) -> std::io::Result<Output> {
    use std::io::Write;
    let mut cmd = Command::new(bin);
    cmd.args(args).envs(env.iter().copied()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
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
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Runs `bin args…` in `dir` on the terminal (an editor or a passphrase prompt can use
/// it), collecting only stderr, where git explains what went wrong.
pub fn run_on_terminal(bin: &str, args: &[String], dir: Option<&Path>) -> std::io::Result<Output> {
    let mut cmd = Command::new(bin);
    cmd.args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::piped());
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let out = cmd.spawn()?.wait_with_output()?;
    Ok(Output {
        code: out.status.code(),
        stdout: String::new(),
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
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Runs `bin args…` with `env` set, attached to the terminal (stdin, stdout and stderr),
/// and returns its exit code. For `git rebase -i`, which talks to the user as it goes.
pub fn run_attached(bin: &str, args: &[String], env: &[(&str, &str)]) -> std::io::Result<Option<i32>> {
    Ok(Command::new(bin).args(args).envs(env.iter().copied()).status()?.code())
}

/// Replaces glance with `bin args…`: it gets the terminal, signals and the exit code.
/// Returns only if it couldn't start. Where there's no `exec`, runs it and exits with its
/// code.
pub fn exec(bin: &str, args: &[std::ffi::OsString]) -> std::io::Error {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    #[cfg(unix)]
    return std::os::unix::process::CommandExt::exec(&mut cmd);
    #[cfg(not(unix))]
    match cmd.status() {
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => e,
    }
}

/// Opens a URL in the browser (or a file in its app). False if that didn't work.
pub fn open(target: &str) -> bool {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    run(opener, &[target.to_string()], None).is_ok_and(|o| o.ok())
}
