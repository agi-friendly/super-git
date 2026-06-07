# super-git Roadmap

This roadmap keeps the project small, testable, and safety-first. Each stage
should be useful before the next stage starts.

## Current Position

The project has a working read-side inspection layer and two write-side safety
flows:

```text
inspect -> preview stage-changes -> execute --plan -> undo --token
inspect -> preview worktree-create -> execute --plan -> undo --token
```

The next stages should expand this lifecycle carefully instead of adding raw Git
wrappers.

## Stage 1: CLI Skeleton And Inspect (implemented)

- Rust workspace
- `super-git-core` and `super-git-cli`
- `super-git` CLI binary
- JSON-first output with `--human`
- `doctor`
- `repo add`
- `repo save`
- `repo list`
- `status`
- `inspect`
- `wt list`
- repository config storage
- worktree/status/state parsing
- inspect guardrails, warnings, risk hint, and next-action candidates

## Stage 2: Preview/Execute/Undo Foundation (implemented for `stage_changes`)

- `preview stage-changes`
- plan hash and state fingerprint
- execute trust boundary
- precondition mismatch handling
- internal write allowlist
- undo token generation
- local undo registry provenance
- index-only undo for staged changes

## Stage 3: Global Config And Saved Repositories

Implemented:

- app home resolver with `SUPER_GIT_HOME` override
- OS-specific config path from `directories::ProjectDirs`
- `config.json` schema version 1
- v0 config migration to v1 in memory
- saved repository registry by worktree family
- registry-backed `repo list`
- worktree parent/name templates
- `{main_path}`, `{repo_name}`, and `{ref_slug}` template variables
- `config validate` for templates and saved repository registry shape
- `config set-worktree-template`
- `repo forget`, including registry-only safety and ambiguous selector checks
- JSON parse-error coverage and human smoke coverage for C5 config/repo commands

Next:

- no shell hooks, copy patterns, or profile system

## Stage 4: Safe Worktree Create

Implemented so far:

- `path_safe_v1` ref slug rendering
- config-derived target path resolver
- target parent creation policy
- target path safety flags for existing paths, Git dir nesting, existing
  worktree nesting, case-insensitive name collisions, and reserved names
- contract checkpoint for `worktree_create` preview
- internal worktree-family snapshot based on Git porcelain data
- `preview worktree-create --ref <ref>`
- `preview worktree-create --repo <id-or-name-or-path> --ref <ref>`
- source ref classification for local branch, tag, commit, remote-tracking
  branch, ambiguous ref, and unknown ref
- remote-tracking branch input is recognized and blocked
- ambiguous branch/tag/remote/commit ref input is blocked
- branch occupancy hard blocks when a branch is already checked out elsewhere
- explicit `execution.status` and structured blocked reasons
- unblocked plans report `executable`
- clear risk and reversibility metadata
- target path resolved from config and frozen in the plan
- no `--force`, `--guess-remote`, `--target`, copy patterns, or shell hooks in
  the first implementation
- `execute` revalidates executable `super-git.plan.v0.2` worktree-create plans
  before creating one linked worktree, including source-ref/ref-policy
  consistency
- worktree-create execution writes a local execution record and returns a
  worktree undo token
- `undo` removes unchanged linked worktrees created by `super-git` only after
  local execution-record provenance, clean target state including ignored files,
  lock/prunable checks, and HEAD/ref drift checks pass
- successful worktree-create undo preserves branch refs and history and removes
  an empty parent directory created by `super-git` only when safe
- full `locked` and `prunable` worktree snapshot parsing

Next:

- richer ambiguous-ref diagnostics with candidate details

## Stage 5: Safe Worktree Remove Preview

Implemented so far:

- C7-0 contract checkpoint for destructive worktree removal preview
- C7-A read-only target resolver/scanner for exact absolute linked-worktree
  paths
- target identity from `git worktree list --porcelain` plus Git directory
  metadata
- block detection for main, bare-primary, current, detached, staged, unstaged,
  untracked, ignored, conflicted, locked, prunable, in-progress, and submodule
  targets
- clean linked worktrees report `execution_status: "preview_only"` in the scan
  result
- `preview worktree-remove --worktree <absolute-linked-worktree-path>`
- exact absolute linked-worktree path only in the first implementation
- no `--current` shortcut in the first implementation
- no `--force`
- no branch, remote-ref, commit, or history deletion
- report process-detection limitations for editors, terminals, development
  servers, and file watchers
- `execution.status: "preview_only"` for clean removable targets until a later
  confirmation/execute contract exists
- `undo_strategy.kind: "not_available"` plus recovery hints instead of
  pretending removal is reversible
- `super-git.confirmation.v0.1` contract for future destructive execute
  authorization
- confirmation is separate from display prompt text and is never enough to skip
  fresh target revalidation
- `execute` parses `super-git.plan.v0.3` `worktree_remove` plans and rejects
  them with `confirmation_required` before any write

Next:

- parse and validate `super-git.confirmation.v0.1` artifacts while still
  refusing to delete

## Stage 6: Repository Profile And Dashboard

- lightweight repository profile for scale/history hints
- repository size, commit count, initial commit, last commit, and remotes where
  performance is acceptable
- clear distinction between cheap always-on fields and expensive opt-in fields
- multi-repository dashboard model
- registered repository summaries
- stable JSON for future UI surfaces

## Stage 7: Guides For Agents

- `super-git guide list`
- conflict/rebase/worktree/super-git usage guides
- documentation-oriented output for weaker or older coding models
- no fake execution magic: guides teach how to use the existing safety contract

## Stage 8: Desktop Prototype

- thin desktop UI over the core/CLI contracts
- repository list
- inspect summary
- status/worktree views
- preview/execute/undo flow visualization

## Stage 9+: Integrations And Advanced Git

- Windows Explorer integration
- macOS Finder integration
- Linux file-manager integration
- plugin or extension system
- conflict helper
- patch create/apply workflow
- interactive rebase helper
- reflog and branch browser
