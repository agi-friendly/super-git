# super-git

super-git은 여러 Git 도구를 사용하며 느꼈던 장점을 모아,
가볍고 기본에 충실한 Git 작업 도구를 만드는 실험 프로젝트다.

## Product Principles

- Git의 기본 동작을 신뢰성 있게 지원한다.
- 모든 핵심 기능은 CLI에서 사용할 수 있어야 한다.
- Desktop UI는 CLI/Core 기능을 감싸는 얇은 레이어로 둔다.
- Windows, macOS, Linux를 지원한다.
- 기본 앱은 가볍게 유지하고, 무거운 기능은 Plugin 형태로 분리한다.
- 여러 저장소를 하나의 프로그램에서 관리할 수 있어야 한다.
- Git worktree 작업을 가장 편하고 안전하게 수행할 수 있어야 한다.
- OS 파일 탐색기에서 바로 실행할 수 있어야 한다.

## Inspiration

- TortoiseGit: Windows Explorer integration, rebase UX, lightweight usage
- Fork: multi-repository management
- IntelliJ IDEA Git: conflict handling, patch, interactive rebase support
- VS Code Git Worktree Manager: worktree creation and workspace opening flow

## Initial Scope

The first version focuses on repository registration, repository status,
worktree listing, worktree creation, worktree removal, and opening worktrees
in external tools such as VS Code, IntelliJ IDEA, or terminal.

## Current CLI

The current implementation starts with a small, AI-first CLI surface.

```bash
super-git doctor
super-git repo add <path>
super-git repo list
super-git status [path]
super-git inspect [path]
super-git preview stage-changes
super-git wt list [path]
```

`super-git inspect` is the flagship command: it surfaces the repository's
hidden state machine as a versioned safety snapshot. The JSON includes HEAD,
worktree-family context, upstream comparison, working-tree summary, in-progress
operation, warnings, a current-state risk hint, and `next` guardrail buckets.
The JSON is self-describing: `summary.execution_permission` is
`not_granted_by_inspect`, `next.execution_contract` is `preview_required`, and
`next.raw_git_allowed` is `false`. Action `reference_command` values are
documentation references, not commands to execute directly.

The CLI binary is named `super-git`. It wraps the installed system `git` command and
keeps repository registration in a simple cross-platform config file.

The next write-side stage is not raw execution. It is the
`inspect -> preview -> execute -> undo` lifecycle. `preview stage-changes` now
emits a validated, read-only plan for staging current unstaged/untracked changes.
`execute --plan <file|->` re-checks that plan, rejects stale or tampered state,
and stages only through the internal `stage_changes` allowlist. `undo --token
<file|->` treats token input as untrusted, validates repository/snapshot/checksum
preconditions, restores the pre-execute index only when the current index still
matches the token, and never edits working-tree files.

## Development Setup

See [docs/setup.md](docs/setup.md) for required tools and OS-specific setup
notes.

## Non-goals for Early Versions

- Reimplementing Git
- Replacing all features of existing Git GUI tools
- Building complex merge/rebase UI from the beginning
- Building a full plugin system before the core workflow is stable
