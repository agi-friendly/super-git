# Getting Started

This guide explains how to build and verify `super-git` from a fresh clone.

## Requirements

- Git
- Rust toolchain
- Cargo
- rustfmt
- Clippy

`super-git` does not reimplement Git. It wraps the installed system `git`
command, so `git --version` must work in the terminal.

## Install Rust

The recommended Rust installation path is `rustup`.

### macOS And Linux

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup component add clippy rustfmt
```

### Windows

Install:

- Git for Windows
- Rust via rustup
- Microsoft C++ Build Tools if Cargo reports a missing linker

Then add the Rust components:

```powershell
rustup component add clippy rustfmt
```

## Verify The Toolchain

```bash
git --version
rustc --version
cargo --version
cargo clippy --version
cargo fmt --version
```

## Clone And Test

```bash
git clone <repository-url>
cd super-git
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Run The CLI

During development, run the binary through Cargo:

```bash
cargo run -p super-git-cli -- doctor
cargo run -p super-git-cli -- inspect
```

The default output is JSON:

```json
{
  "ok": true,
  "data": {
    "arch": "aarch64",
    "config_path": "/Users/<name>/Library/Application Support/com.super-git.super-git/config.json",
    "git_version": "git version 2.54.0",
    "os": "macos"
  }
}
```

Use `--human` for terminal reading:

```bash
cargo run -p super-git-cli -- --human inspect
```

## Try The Safety Loop

Use a disposable repository or throwaway worktree for first experiments.

```bash
cargo run -p super-git-cli -- inspect
```

After making an unstaged or untracked test change:

```bash
cargo run -p super-git-cli -- preview stage-changes > /tmp/super-git-plan.json
cargo run -p super-git-cli -- execute --plan /tmp/super-git-plan.json > /tmp/super-git-result.json
cargo run -p super-git-cli -- undo --token /tmp/super-git-result.json
```

Important details:

- `inspect` is read-only.
- `preview stage-changes` creates a plan but does not stage files.
- `execute` revalidates the plan and state before staging.
- `undo` validates local provenance before restoring the previous index.
- `undo` does not modify working-tree file contents.

## Runtime Config Location

`super-git repo save [path]` stores registered repository families in an
OS-specific config directory. `repo add <path>` is still accepted as a
compatibility alias. Run `doctor` to see the exact config path:

```bash
cargo run -p super-git-cli -- doctor
```

Approximate locations:

- macOS: `~/Library/Application Support/com.super-git.super-git/config.json`
- Linux: `~/.config/super-git/config.json` or the XDG equivalent
- Windows: `%APPDATA%` equivalent chosen by the `directories` crate

## Optional Developer Tools

These are useful but not required:

- rust-analyzer
- RustRover or VS Code
- Git GUI tools for comparison and manual inspection

Desktop UI dependencies such as Tauri, Node.js, Svelte, or Electron are not
required for the current CLI/core work.
