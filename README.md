# super-git

AI-first Git safety tooling for humans and coding agents.

`super-git` is a CLI-first experiment that makes Git's hidden state machine
explicit before any write action happens. The core product contract is:

```text
inspect -> preview -> execute -> undo
```

Undo is action-specific, not promised for every write. Destructive flows must
use explicit confirmation and recovery contracts when automatic undo cannot be
honestly proven.

The tool is designed for AI agents first, but the same properties make it useful
for humans: clear state, structured output, dry-run planning, guarded execution,
and undo provenance.

## Why This Exists

Git is powerful, but its critical state is spread across status output, internal
files, reflog, index state, worktree metadata, and command-specific edge cases.
Humans often rely on IDEs or GUI tools to keep that state visible. Coding agents
usually operate from a terminal and must reconstruct the state from scratch.

`super-git` aims to give both humans and agents a safer contract:

- See the repository state in one structured snapshot.
- Preview write actions before changing the repository.
- Execute only validated plans, not arbitrary command strings.
- Undo supported writes through local provenance checks.
- Keep JSON as the default output, with `--human` for terminal reading.

## Current Status

This project is still early, but the first safety loop exists.

Implemented today:

- `super-git inspect [path]`
  - repository root
  - worktree family context
  - HEAD and detached/unborn state
  - upstream ahead/behind
  - working-tree summary and conflicts
  - in-progress Git operation
  - warnings, risk hint, summary, and guarded next-action candidates
- `super-git preview stage-changes`
  - builds a read-only plan for staging current unstaged/untracked changes
- `super-git preview worktree-create --ref <ref> [--repo <selector>]`
  - builds a read-only plan for creating a linked worktree
  - recognizes blocked cases such as occupied branches, remote-tracking refs,
    and target collisions without writing
- `super-git preview worktree-remove --worktree <absolute-linked-worktree-path>`
  - builds a read-only destructive-action plan for removing an existing linked
    worktree
  - reports strict hard blocks, human confirmation requirements, no automatic
    undo, and manual recovery hints without deleting anything
- `super-git execute --plan <file|-> [--confirmation <file|->]`
  - re-validates the plan and state before writing
  - executes only internal allowlisted actions: `stage_changes`, executable
    `worktree_create` plans, and confirmed `worktree_remove` plans
  - writes local provenance before reporting success
  - for destructive `worktree_remove`, requires a separate confirmation
    artifact and returns no automatic undo token
- `super-git undo --token <file|->`
  - treats token input as untrusted
  - for `stage_changes`, validates repository, snapshot checksums, current
    index checksum, and local registry provenance before restoring the
    pre-execute index
  - for `worktree_create`, validates local execution-record provenance, target
    worktree identity, clean state, lock state, and HEAD/ref drift before
    removing the linked worktree
  - never edits working-tree file contents or deletes branch refs/history
- Supporting commands: `doctor`, `config path`, `config show`,
  `config validate`, `config set-worktree-template`, `repo save`, `repo add`,
  `repo list`, `repo forget`, `status`, `wt list`

## Quick Start

```bash
cargo run -p super-git-cli -- doctor
cargo run -p super-git-cli -- inspect
```

In a disposable repository or throwaway worktree with unstaged/untracked
changes:

```bash
cargo run -p super-git-cli -- preview stage-changes > /tmp/super-git-plan.json
cargo run -p super-git-cli -- execute --plan /tmp/super-git-plan.json > /tmp/super-git-result.json
cargo run -p super-git-cli -- undo --token /tmp/super-git-result.json
```

By default, every command returns a JSON envelope:

```json
{ "ok": true, "data": {} }
```

Failures use the same contract:

```json
{ "ok": false, "error": { "message": "...", "causes": [] } }
```

Use `--human` when reading directly in a terminal:

```bash
cargo run -p super-git-cli -- --human inspect
```

## Documentation

- [Documentation map](docs/README.md)
- [Getting started](docs/getting-started.md)
- [Command reference](docs/command-reference.md)
- [Safety model](docs/safety-model.md)
- [Architecture](docs/architecture.md)
- [Roadmap](docs/roadmap.md)
- [ADR 0001: CLI First](docs/adr/0001-cli-first.md)
- [Commit messages](docs/contributing/commit-messages.md)
- [Archived original notes](docs/archive/original-notes/README.md)

## Development

Required tools:

- Git
- Rust toolchain
- Cargo
- rustfmt
- Clippy

Verification:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Non-goals For Early Versions

- Reimplementing Git
- Replacing mature Git GUI tools
- Running raw Git commands from `inspect` suggestions
- Building a desktop UI before the CLI/core contract is stable
- Building a plugin system before the safety lifecycle proves itself

Desktop and GUI ideas are still part of the long-term dream. For now, the
project is building the safe core that those interfaces can later wrap.
