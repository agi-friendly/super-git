# Documentation

This directory contains the current product documentation for `super-git`.

## Recommended Reading Order

1. [Getting started](getting-started.md)
2. [Command reference](command-reference.md)
3. [Safety model](safety-model.md)
4. [Architecture](architecture.md)
5. [Roadmap](roadmap.md)
6. [ADR 0001: CLI First](adr/0001-cli-first.md)
7. [Commit messages](contributing/commit-messages.md)

## Current Source Of Truth

- `README.md` introduces the project for humans discovering it on GitHub.
- `AGENTS.md` tells new coding-agent sessions how to work safely in this repo.
- `docs/safety-model.md` defines the active safety contract.
- `docs/command-reference.md` describes the current CLI behavior.
- `docs/roadmap.md` tracks planned direction.
- `docs/contributing/commit-messages.md` defines the commit history standard.

## Internal Planning

Implementation plans and design checkpoints live under:

- [internal/plans](internal/plans/)
- [Global config and saved repositories design](internal/plans/2026-06-06-global-config-and-saved-repositories.md)
- [Worktree create preview contract](internal/plans/2026-06-07-c6-0-worktree-create-preview-contract.md)
- [Worktree remove preview contract](internal/plans/2026-06-07-c7-0-worktree-remove-preview-contract.md)
- [Worktree remove confirmation contract](internal/plans/2026-06-07-c7-c-worktree-remove-confirmation-contract.md)

These files are useful for project history and implementation context, but the
public-facing contract should be reflected in the main docs listed above.

## Archive

Original research and dual-brain notes moved to:

- [archive/original-notes](archive/original-notes/README.md)

They are preserved intentionally, but they may describe earlier goals such as a
worktree-first desktop tool. The current direction is AI-first Git safety:

```text
inspect -> preview -> execute -> undo
```

Undo is action-specific. Destructive flows that cannot honestly prove
reversibility use explicit confirmation and recovery hints instead of an
automatic undo token.
