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
- `super-git config validate`
- `super-git config set-worktree-template [options]`
- `super-git repo save [path]`
- `super-git repo add <path>`
- `super-git repo list`
- `super-git repo forget <id-or-name-or-path>`
- `super-git status [path]`
- `super-git inspect [path]`
- `super-git preview stage-changes`
- `super-git preview worktree-create --ref <ref> [--repo <id-or-name-or-path>]`
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

Write actions enter through a staged safety lifecycle. The full loop for
undoable actions is:

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

`undo --token <file|->` is action-specific. `stage_changes` undo validates
token schema, repository identity, snapshot checksum, current index checksum,
and local undo registry provenance before restoring the previous index
snapshot. `stage_changes` undo never edits working-tree file contents.

`worktree_create` undo has a different boundary: it validates local execution
record provenance, confirms the target is still the clean linked worktree
created by `super-git`, refuses locked/prunable/main/dirty/drifted targets, and
then removes the linked worktree with `git worktree remove` without `--force`.
It must not delete branch refs, remote refs, commits, history, or user-created
files.

Not every Git write can honestly offer automatic undo. Destructive actions such
as removing an existing linked worktree must say so in their preview contract,
stay preview-only until confirmation and revalidation are designed, and provide
recovery hints instead of pretending to be reversible.

## Config

Registered repositories are stored in a JSON file under the resolved
`super-git` app home. `super-git-core::config::store::ConfigStore` owns this
storage.

The app home now resolves from `SUPER_GIT_HOME` first, then from the
OS-specific `ProjectDirs` config location. `super-git config path` reports the
resolved home, source, and config file. `super-git config show` reports the same
location plus the currently loaded config. `super-git config validate` reports
validation issues without writing the file.

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
The identity hash preserves path case so case-sensitive filesystems do not
collapse different repository families that differ only by case.
`repo add <path>` remains as a compatibility alias for `repo save <path>`.
Bare-primary families are supported with `kind: "bare_worktree_family"` and
`main_worktree: null`.

`repo forget <id-or-name-or-path>` removes only the saved registry entry. It
never deletes repository directories, linked worktrees, bare Git directories, or
working-tree files. Selectors match full repository id, path-like selectors, or
unique repository name. Ambiguous selectors fail without rewriting the config
file, including cases where the same token points to different repositories by
different selector kinds.

Worktree template settings can be edited with
`config set-worktree-template`. Template variables use braces, not shell syntax.
C5 supports `{main_path}`, `{repo_name}`, and `{ref_slug}` plus the
`path_safe_v1` slug algorithm name. `config validate` also checks saved
repository registry shape: ids must match the saved `git_common_dir`, path fields
must be absolute, ids must be unique, and bare-primary entries must not claim a
`main_worktree`. The template command validates field-specific rules before
saving:

- `parent_template` must contain `{main_path}` exactly once and must not contain
  `{ref_slug}` or a literal `..` path component.
- `name_template` must contain `{ref_slug}` exactly once and must not contain
  `{main_path}` or path separators.
- `ref_slug_algorithm` currently supports only `path_safe_v1`.

Config is preview input, not execute authority. Future worktree preview commands
must resolve and freeze paths into plans; execute should not re-expand template
strings as trusted instructions.

Shell hooks, copy patterns, and multiple profiles are out of scope until the
core safety lifecycle has explicit preview and confirmation rules for them.

## Worktrees

Worktree management remains an important differentiator. The current foundation
is read-first and safety-gated:

- `inspect.worktree_context` shows where the current repository sits in its
  worktree family.
- `wt list` returns the full worktree list.

Create/remove workflows must start with preview plans and safety checks before
they become executable. Worktree creation is a typed `worktree_create` plan,
not a raw `git worktree add` command string.

`preview worktree-create` is read-only. It resolves a repository family, source
ref, config-derived target path, family snapshot, branch occupancy, execution
status, risk, and undo boundary into `super-git.plan.v0.2`. Unblocked plans
report `execution.status: "executable"`, but execution still revalidates plan
hash, repository family identity, source ref commit, source-ref/ref-policy
consistency, family snapshot, branch occupancy, and target path safety
immediately before writing. Blocked Git-state cases such as remote-tracking
branch input, ambiguous ref input, occupied local branches, and target
collisions return useful `{ ok: true, data.execution.status: "blocked" }`
plans. Repository selector failures still return `{ ok: false, error }`
because no family-specific plan can be formed.

The first worktree create contract intentionally avoids `--force`,
`--guess-remote`, target overrides, copy patterns, and shell hooks. Preview
should resolve the repository family, source ref, target path, target safety,
branch occupancy, and undo boundary before execution is possible. The detailed
contract is recorded in
`docs/internal/plans/2026-06-07-c6-0-worktree-create-preview-contract.md`.

`undo` supports unchanged `worktree_create` results by consuming the worktree
undo token from `execute`. The execution record under the Git common directory
is required provenance; partial or tampered records are not cleanup permission.
Successful undo removes only the linked worktree and an empty parent directory
created by `super-git`, and refuses ignored files as well as tracked or
untracked changes before removal.

Worktree removal has a different boundary. Removing an existing linked
worktree is a destructive action, not an undoable cleanup action. The C7
contract starts with `preview worktree-remove` only: exact absolute
linked-worktree path, read-only target scanning, no `--force`, no
branch/history deletion, no automatic undo, and human confirmation required
before any future execute support. The detailed checkpoint is recorded in
`docs/internal/plans/2026-06-07-c7-0-worktree-remove-preview-contract.md`.

## Plugins And Guides

Plugin or guide systems are future work. They should not appear before the core
safety lifecycle is stable enough to teach.

First, the project should prove which Git workflows deserve first-class support
and which can be delivered as documentation, guides, or optional extensions.
