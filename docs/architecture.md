# super-git Architecture

`super-git` starts as a CLI-first, AI-first Git safety layer. The first goal is
not to ship a desktop app quickly. The first goal is to build a small, reliable
core that exposes Git state and guards write actions.

## Layers

### Core Library

`super-git-core` owns Git command execution, repository validation, config
storage, status/worktree parsing, repository inspection, preview plan creation,
execute validation, and undo validation.

The core library does not know about terminal rendering or desktop UI. CLI,
future GUI surfaces, and possible file-manager integrations should all wrap the
same core contracts.

### CLI

`super-git-cli` provides the `super-git` binary. Every core workflow should work
from the CLI before another UI wraps it.

Output is JSON-first. Success uses `{ ok, data }`; failure uses `{ ok, error }`.
`--human` switches to terminal-friendly rendering.

Current commands:

- `super-git doctor`
- `super-git config path`
- `super-git config show`
- `super-git repo save [path]`
- `super-git repo add <path>`
- `super-git repo list`
- `super-git status [path]`
- `super-git inspect [path]`
- `super-git preview stage-changes`
- `super-git execute --plan <file|->`
- `super-git undo --token <file|->`
- `super-git wt list [path]`

### Desktop UI

Desktop UI is a later thin layer over the core/CLI contracts. Tauri + Svelte is
one possible future direction, but not a current implementation target.

The UI should not become a second Git implementation. It should wrap the same
validated inspect/preview/execute/undo flow.

## Git Strategy

The project does not reimplement Git. It calls the installed system `git`
through `std::process::Command`.

Command arguments are passed as arrays instead of shell command strings. This
keeps paths with spaces safer and avoids shell interpretation.

## Inspect Contract

`inspect` is read-only.

It returns a versioned safety snapshot with repository root, worktree context,
HEAD, upstream comparison, working-tree summary, in-progress operation, warnings,
risk hint, summary, and guarded next-action candidates.

`inspect` does not grant execution permission:

- `summary.execution_permission` is `not_granted_by_inspect`.
- `next.execution_contract` is `preview_required`.
- `next.raw_git_allowed` is `false`.
- action `reference_command` values are documentation references.

## Preview/Execute/Undo Contract

Write actions grow only through:

```text
inspect -> preview -> execute -> undo
```

`preview` reads current state and creates a plan with action-specific
preconditions, state fingerprint, resolved paths, risk metadata, and undo
strategy. The plan is a contract, not a script. `reference_commands` are
explanatory and cannot be executed by `execute`.

`execute --plan <file|->` validates schema, plan hash, action kind, options,
repository root, fingerprint, and resolved pathset immediately before writing.
State drift becomes `precondition_mismatch`. Actual Git commands are rebuilt
from the core allowlist.

`undo --token <file|->` validates token schema, repository identity, snapshot
checksum, current index checksum, and local undo registry provenance before
restoring the previous index snapshot. It never edits working-tree file
contents.

## Config

Registered repositories are stored in a JSON file under the resolved
`super-git` app home. `super-git-core::config::store::ConfigStore` owns this
storage.

The app home now resolves from `SUPER_GIT_HOME` first, then from the
OS-specific `ProjectDirs` config location. `super-git config path` reports the
resolved home, source, and config file. `super-git config show` reports the same
location plus the currently loaded config.

`config.json` uses schema version 1:

```json
{
  "schema_version": 1,
  "settings": {
    "worktree": {
      "parent_template": "{main_path}.worktrees",
      "name_template": "{repo_name}__{ref_slug}",
      "ref_slug_algorithm": "path_safe_v1"
    }
  },
  "repositories": [
    {
      "id": "sha256:<git-common-dir-identity>",
      "name": "naon-dnl",
      "kind": "worktree_family",
      "main_worktree": "/path/to/naon-dnl",
      "git_common_dir": "/path/to/naon-dnl/.git",
      "saved_from": "/path/to/naon-dnl.worktrees/naon-dnl__feature"
    }
  ]
}
```

Legacy v0 files without `schema_version` are migrated in memory. Legacy entries
that no longer resolve to Git repositories are skipped because they cannot be
assigned a worktree-family identity. Saving always writes the current v1 shape
using the existing atomic write pattern. Unknown future schema versions fail
instead of being partially interpreted.

Saved repositories are stored as worktree families, not individual linked
worktrees. `repo save [path]` uses Git's common directory as the family identity
so saving the main worktree and a linked worktree deduplicates to one entry.
`repo add <path>` remains as a compatibility alias for `repo save <path>`.
Bare-primary families are supported with `kind: "bare_worktree_family"` and
`main_worktree: null`.

The next config milestones are template editing and repository removal, not a
full user profile system. The planned direction is:

- Use worktree templates such as `{main_path}.worktrees` and
  `{repo_name}__{ref_slug}`.
- Treat config as preview input, not execute authority.

Shell hooks, copy patterns, and multiple profiles are out of scope until the
core safety lifecycle has explicit preview and confirmation rules for them.

## Worktrees

Worktree management remains an important differentiator. Current functionality
is intentionally read-oriented:

- `inspect.worktree_context` shows where the current repository sits in its
  worktree family.
- `wt list` returns the full worktree list.

Create/remove workflows should start with preview plans and safety checks before
they become executable.

## Plugins And Guides

Plugin or guide systems are future work. They should not appear before the core
safety lifecycle is stable enough to teach.

First, the project should prove which Git workflows deserve first-class support
and which can be delivered as documentation, guides, or optional extensions.
