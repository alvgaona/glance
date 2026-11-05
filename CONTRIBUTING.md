# Contributing to glance

Thanks for helping. Bugs and ideas go in [issues](https://github.com/alvgaona/glance/issues);
the [roadmap](https://github.com/alvgaona/glance/issues/1) lists what's planned. For
anything bigger than a small fix, open an issue first so we can agree on the shape.

## Setup

You need git and Rust 1.88 or newer. The dev tools (lefthook, commitlint, git-cliff,
and a Rust toolchain) come from [pixi](https://pixi.sh):

```sh
pixi install       # the tools, in .pixi/ inside the repo
pixi run hooks     # git hooks: fmt + clippy before a commit, commitlint on the message
```

Without pixi, plain cargo works; install lefthook and commitlint yourself if you want
the hooks.

| Task | pixi | cargo |
| --- | --- | --- |
| Build | `pixi run build` | `cargo build --release` |
| Test | `pixi run test` | `cargo test` |
| Format | `pixi run fmt` | `cargo fmt` |
| Lint (both builds) | `pixi run lint` | `cargo clippy --all-targets [--no-default-features] -- -D warnings` |
| Changelog | `pixi run changelog` | `git-cliff --output CHANGELOG.md` |

## Two builds

The `tui` feature (on by default) adds the full-screen views. Without it, glance only
prints. Both have to build, lint clean and pass tests:

```sh
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
```

## How the code is laid out

Each module has one job, and the boundaries are kept on purpose:

- `git/` reads the repository through [gix](https://github.com/GitoxideLabs/gitoxide)
  and owns every write. Writes run the `git` binary (`git/run.rs`), so hooks, signing
  and config behave exactly as with git itself. Nothing outside `git/` builds git
  arguments.
- `print/` is the printed output (piped, or `-l` flags). `tui/` is the full-screen
  side. **They never import each other**; shared text helpers live in `fmt/`.
- `cmd/` has one file per command: it decides between printing and a screen.
- `tui/kit/` holds generic widgets (lists, dialogs, toasts, palette, text areas),
  `tui/panels/` git-aware pieces shared by screens (file lists, diffs), and
  `tui/screens/<name>/` the screens, split into `mod.rs`, `keys.rs` and `draw.rs`.
  All screens run on the one event loop in `tui/app.rs`.

Side effects live at the edges; everything else is plain data in and out:

- **Only `process.rs` starts programs.** clippy bans `Command::new` elsewhere.
- **Writes return `Ran`** (`Result<String, String>`, git's output or its error), so a
  screen shows it with `Toast::ran`.
- **Screens: keys change state, `apply` touches the world.** A key handler never does
  I/O; it returns `Flow::Run(effect)`, and the screen's one `apply` runs it (git,
  clipboard, browser). Effects that need the terminal (git may prompt, an editor opens)
  say so in `in_terminal`. Lazy loads and background results go in `tick`; `draw` only
  reads state. A confirm dialog stores the effect it confirms.
- **No hidden shared state.** clippy bans `RefCell`, `Cell`, `Mutex` and `RwLock`;
  background work sends its result over a channel.
- **Iterator chains over loop-and-push**, except where a loop keeps running state,
  exits early or uses `?`.
- Printers build `lines()` and `print` writes them, so output is testable.

The screens follow one style: full screen, borderless panels, a footer with the main
keys, `ctrl+p` (or `:`) for every command, toasts for results, a dialog before
anything that can't be undone.

## Tests

`cargo test` runs unit tests, and every git write is tested on a scratch repository
made in the temp folder (see the `Scratch` helper in `git/stage.rs`). Tests mustn't
change the process's working folder: they run in parallel.

For a screen, also try it by hand on a scratch repository (merges, conflicts, detached
HEAD, worktrees, a repo with no commits) and in a small terminal.

## Commits

[Conventional commits](https://www.conventionalcommits.org): `feat:`, `fix:`,
`refactor:`, `perf:`, `docs:`, `test:`, `build:`, `ci:`, `chore:`, with an optional
scope (`feat(add): …`). Lowercase subject, no trailing period. The commit-msg hook and
CI check this, and the changelog is built from these messages, so write them for the
person reading the release notes.

## Releases

1. Bump `version` in `Cargo.toml` and run `cargo check` (it updates `Cargo.lock`).
2. `pixi run changelog`, read it, commit as `chore(release): vX.Y.Z`.
3. Tag `vX.Y.Z` and push the tag.

The release workflow builds macOS and Linux binaries, makes the GitHub release with
the notes for that tag, and publishes the crate. A tag like `v0.2.0-rc.1` makes a
pre-release on GitHub only.
