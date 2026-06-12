# super-git Roadmap

This roadmap keeps the project small, testable, and safety-first. Each stage
should be useful before the next stage starts.

## Current Position

The project has a working read-side inspection layer, three undoable write-side
safety flows, and one confirmed destructive flow:

```text
inspect -> preview stage-changes -> execute --plan -> undo --token
inspect -> preview worktree-create -> execute --plan -> undo --token
inspect -> preview history-edit -> execute --plan [--confirmation] -> undo --token
inspect -> preview worktree-remove -> execute --plan --confirmation
```

`worktree_remove` is intentionally not automatically undoable; it requires
explicit confirmation and recovery hints instead of an `undo_token`.
`history_edit` requires the confirmation artifact only when the range is
published, and its undo restores the pre-execute branch tip locally.

The next stages should expand this lifecycle carefully instead of adding raw Git
wrappers.

Direction note (2026-06-10): capability stages come before observation
surfaces. Reading Git state is already easy for agents; editing history,
predicting conflicts, and driving the lifecycle without juggling plan files are
not. The repository profile and dashboard move after the capability stages so
future dashboards can be designed around real super-git tools instead of
generic Git summaries.

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

## Stage 5: Safe Worktree Remove

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
- `execution.status: "preview_only"` for clean removable targets, with
  execution allowed only after a separate confirmation artifact
- `undo_strategy.kind: "not_available"` plus recovery hints instead of
  pretending removal is reversible
- `super-git.confirmation.v0.1` contract for destructive execute
  authorization
- confirmation is separate from display prompt text and is never enough to skip
  fresh target revalidation
- `execute` parses `super-git.plan.v0.3` `worktree_remove` plans and rejects
  missing confirmation with `confirmation_required` before any write
- `execute --confirmation <file|->` parses and statically validates
  `super-git.confirmation.v0.1` artifacts, revalidates the target, writes an
  execution record, and removes only the linked worktree without `--force`
- successful `worktree_remove` execute omits `undo_token` and records
  `automatic_undo_available: false`

Next:

- extend worktree remove coverage with additional stale-state fixtures before
  expanding into remove cleanup workflows

## Stage 6: History Edit (Plan-Based Interactive Rebase)

Why this is next:

- `git rebase -i` depends on an interactive editor, which most agent harnesses
  cannot open. Agents fall back to fragile `GIT_SEQUENCE_EDITOR` scripting, and
  that is a top history-loss failure mode for weaker models.
- History editing is where IDE users gain the most over plain CLI users:
  rewording intermediate commits, squashing, dropping, and reordering. Agents
  deserve the same leverage through a structured contract.
- Unlike worktree removal, history edit has an honestly provable undo: rebase
  moves a branch pointer without deleting objects, so a pre-execute branch
  snapshot can restore the previous tip.

Implemented so far:

- C8-0 contract checkpoint for plan-based history edit
  (`docs/internal/plans/2026-06-10-c8-0-history-edit-preview-contract.md`)
- C8-A read-only range resolver, published scan, and instruction validation
  with structured block codes in `super-git-core`
- C8-B `preview history-edit` command emitting `super-git.plan.v0.4` with
  survey/executable/preview_only/blocked execution states and a stable plan id
- C8-C execute for unpublished plans: plumbing-rebuilt commits, compare-and-swap
  branch move, tree-identity post-verify with rollback, author preservation, and
  a branch-tip-snapshot undo token
- C8-D undo: provenance-checked branch-tip restore by compare-and-swap, refusing
  an advanced tip and verifying no other ref moved
- C8-E confirmation-gated execute for published ranges: a matching
  `super-git.confirmation.v0.1` artifact with the deterministic phrase plus fresh
  revalidation; the local undo token still applies but cannot un-publish

Planned shape:

- `preview history-edit` over a commit range, returning per-commit subject,
  full message, author, upstream/pushed status, and edit constraints as JSON
- calling preview without instructions returns a read-only survey that
  doubles as the instruction-list template, so weaker agents never
  reconstruct history from `git log` parsing by hand
- a declarative instruction list inside the plan: `pick`, `reword`, `squash`,
  `fixup`, `drop`, and reordering, instead of todo-file strings
- the first op set never changes any tree, so `execute` rebuilds the commit
  chain with Git plumbing and moves the branch ref atomically; interactive
  rebase machinery is never invoked, and plan-provided text is never executed
  directly
- hard blocks first: in-progress operations, conflicted paths, and merge
  commits in range; staged and unstaged changes are allowed with a warning
  because the mechanism never touches files
- rewriting commits already on an upstream requires the destructive
  confirmation contract from Stage 5 instead of a silent allow
- undo token restores the pre-execute branch tip after provenance checks

Slicing direction:

- start with `reword` plus `squash`/`fixup`, which cannot produce content
  conflicts
- `drop` and reordering land after conflict prediction exists (Stage 7)
- commit `split` is intentionally deferred

## Stage 7: Merge And Rebase Conflict Prediction

Contract checkpoint:
`docs/internal/plans/2026-06-12-c9-0-conflict-prediction-contract.md` (C9-0).
Done so far: the C9-A merge prediction core
(`super-git.conflict-prediction.v0.1`), the C9-B `predict merge` CLI verb,
and the C9-C rebase-chain prediction core
(`super-git.rebase-prediction.v0.1`, per-step replay that stops at the first
predicted conflict). The rebase CLI verb, inspect integration, and the
`drop`/reorder consumer are open.

- `git merge-tree`-based dry-run prediction for merge and rebase previews
- per-file predicted conflicts with both contributing commits
- prediction feeds Stage 6 `drop`/reorder steps and standalone merge or rebase
  previews; a safe `drop` for wip commits is the highest-demand consumer and
  should land early
- `inspect` gains the branch-relationship context prediction needs, such as
  merge-base and shared-upstream hints
- safe branch refresh from
  `docs/internal/idea/2026-06-07-safe-branch-refresh.md` belongs to this
  family: fast-forward-only branch updates through existing or temporary
  worktrees
- no automatic conflict resolution; prediction and guidance only

## Stage 8: Graduated Execution For Agents

- one-shot execution for low-risk reversible actions, running the same
  preview -> validate -> execute pipeline in a single invocation
- two-phase plan and confirmation flows stay mandatory for destructive or
  history-rewriting actions
- plan piping via stdin/stdout documented as the default chaining pattern, so
  weaker agents do not juggle temp files
- error payloads include machine-actionable next-step candidates, mirroring
  `inspect.next` guardrails

## Stage 9: Repository Profile And Dashboard

Deliberately placed after the capability stages: the dashboard should be
imagined from super-git's own contracts, such as history edit, conflict
prediction, and graduated execution, not from generic Git summaries.

- lightweight repository profile for scale/history hints
- repository size, commit count, initial commit, last commit, and remotes where
  performance is acceptable
- clear distinction between cheap always-on fields and expensive opt-in fields
- multi-repository dashboard model
- registered repository summaries
- stable JSON for future UI surfaces

## Stage 10: Guides For Agents

- `super-git guide list`
- conflict/rebase/worktree/super-git usage guides
- documentation-oriented output for weaker or older coding models
- no fake execution magic: guides teach how to use the existing safety contract

## Stage 11: Desktop Prototype

- thin desktop UI over the core/CLI contracts
- repository list
- inspect summary
- status/worktree views
- preview/execute/undo flow visualization

## Stage 12+: Integrations And Advanced Git

- Windows Explorer integration
- macOS Finder integration
- Linux file-manager integration
- plugin or extension system
- conflict helper
- patch create/apply workflow
- reflog and branch browser
