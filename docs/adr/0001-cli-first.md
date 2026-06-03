# ADR 0001: CLI First

## Status

Accepted

## Context

The long-term dream is a Git tool that learns from Fork, TortoiseGit, IntelliJ
IDEA Git, and VS Code Git Worktree Manager.

The project direction has since become sharper: build an AI-first safety layer
first, then let human-facing UI wrap it later.

Starting with a desktop app, file-manager integration, plugins, and advanced
merge/rebase UI would create too many failure points before the Git workflow
contract is stable.

## Decision

`super-git` starts CLI-first.

- Separate the core library from the CLI.
- Provide the CLI as the `super-git` binary.
- Make JSON the default output and `--human` the opt-in rendering.
- Add desktop UI only after the core/CLI contract is stable.
- Wrap the system `git` command before attempting deeper Git implementation.
- Postpone plugins until the core workflow proves itself.

## Why CLI First

CLI is easy to test and easy to debug. It also works naturally for coding
agents, CI, scripts, and humans who want a precise terminal tool.

If every core workflow works from the CLI, future desktop tools can wrap the
same behavior without creating a second source of truth.

## Why Rust

Rust is a good fit for cross-platform CLI/core work. It makes single-binary
distribution realistic and handles paths/process execution with strong types.

## Why Not Desktop First

Desktop-first development would force UI state, packaging, and OS-specific
behavior into the project before the Git safety model is proven.

The project should first prove:

```text
inspect -> preview -> execute -> undo
```

## Why Wrap System Git First

`super-git` does not reimplement Git. The early versions call the installed
system `git` safely.

Rust uses `std::process::Command` with argument arrays instead of shell command
strings. This is safer for paths, spaces, and cross-platform behavior.

## Why Postpone Plugins

Plugin systems are powerful, but they make early architecture heavy.

The project should first implement the core workflows directly, then decide
which extension points are real.
