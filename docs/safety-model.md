# Safety Model

`super-git` treats Git as a powerful state machine that should be inspected and
validated before writes happen.

The full lifecycle for undoable actions is:

```text
inspect -> preview -> execute -> undo
```

## Principles

### JSON First

Automation should not scrape prose when structured data is available.

- JSON is the default.
- `--human` is opt-in.
- Success and failure both use predictable envelopes.

### Inspect Is Read-only

`inspect` answers:

- Where am I?
- Which worktree family am I in?
- What is HEAD?
- Is the repo ahead or behind upstream?
- Are there staged, unstaged, untracked, or conflicted paths?
- Is a Git operation in progress?
- What actions look safe enough to preview next?

`inspect` never grants execution permission.

### Preview Produces A Contract

`preview` creates a plan with action kind, preconditions, state fingerprint,
resolved paths, risk metadata, and action-specific undo or recovery strategy.

The plan is not a shell script. Documentation fields such as
`reference_commands` must not be executed directly.

### Execute Revalidates

`execute` must re-check the plan before writing:

- schema version
- plan hash
- supported action kind
- supported options
- repository root
- operation state
- fingerprint
- resolved pathset

Git commands are rebuilt from internal allowlists, not copied from the plan.

### Undo Requires Provenance

`undo` treats token input as untrusted.

For the current `stage_changes` action, undo is index-only:

- validate token schema
- validate repository identity
- validate snapshot path
- validate snapshot checksum
- validate local undo registry record created by `execute`
- validate current index checksum
- restore the pre-execute index snapshot only if the current index still matches
  the execute result

`undo` does not edit working-tree files.

For the current `worktree_create` action, undo is removal-only for the linked
worktree created by `super-git`:

- validate token schema
- validate repository family identity
- validate the local execution record under the Git common directory
- require a completed execution record whose undo token matches the provided
  token
- require the target to still be a linked, unlocked, non-prunable worktree
- require the target HEAD/ref to match the execute record
- require no in-progress Git operation and a clean target working tree,
  including ignored files
- remove the linked worktree with `git worktree remove` without `--force`
- remove a parent directory created by `super-git` only if it is empty

`worktree_create` undo does not delete branch refs, remote refs, commits,
history, dirty files, untracked files, ignored files, locked worktrees, or main
worktrees.

## Current Write Boundary

Four Git write actions exist today:

```text
stage_changes
worktree_create
worktree_remove
history_edit
```

The `predict` verbs (`predict merge`, `predict rebase`) are deliberately
outside this boundary: they are reads with no plan, no execute, and no undo,
and they never perform automatic conflict resolution. Their
`git merge-tree --write-tree` backend touches no refs, index, working-tree,
or config state, though it may write unreferenced, gc-collectable objects
into the object database (`predict rebase` additionally wraps each clean
step's tree in an unreferenced synthetic commit) — the safety docs state
that nuance instead of claiming an unqualified "read-only".

`stage_changes` stages the unstaged/untracked pathset captured by `preview`,
but only after `execute` confirms that the pathset and fingerprint still match.

`worktree_create` creates one linked worktree from an executable
`super-git.plan.v0.2`, but only after `execute` revalidates plan hash, source
ref, ref-policy consistency, repository family identity, family snapshot, branch
occupancy, target path safety, and post-create HEAD/ref state. It writes a local
execution record before Git may mutate worktree metadata.

`worktree_remove` removes one existing linked worktree from a confirmed
`super-git.plan.v0.3`, but only after `execute` validates a separate
`super-git.confirmation.v0.1` artifact, revalidates the target immediately
before deletion, writes an execution record, and confirms the command is not
being run from inside the target worktree. It is destructive and does not
return an automatic undo token.

`history_edit` rewrites the `base..HEAD` range of the current branch from a
`super-git.plan.v0.4` built out of declarative `pick`/`reword`/`squash`/`fixup`
instructions. Execute re-derives a fresh plan from the live repository and
requires its plan id to match (so author identity and messages cannot be forged
through a tampered plan), rebuilds the commits with `git commit-tree` while
preserving each original author, writes an intent record, then moves the branch
ref with a compare-and-swap against the pre-execute tip. A post-verify check
proves the final tree is identical to the pre-execute tree; any failure after
the ref moved rolls the branch back. Published ranges are `preview_only` and
require a separate `super-git.confirmation.v0.1` artifact with a typed phrase.
Undo restores the pre-execute branch tip from the execution record, again via
compare-and-swap, and refuses if the branch has advanced since execute. It
moves only the branch pointer: the working tree and index are untouched, and
local undo cannot un-publish history that was already pushed.

Future actions must earn their way into the allowlist with tests and docs.

Global config and repository-registry writes are separate from Git repository
writes. For example, `repo forget` edits only `super-git`'s config registry and
must not delete repository directories, worktrees, bare Git directories, `.git`,
or working-tree files. Ambiguous `repo forget` selectors fail before writing,
including cross-kind matches such as one repository id and another repository
name. Because saved repositories become preview input for later worktree
actions, `config validate` treats malformed registry entries as invalid instead
of silently accepting arbitrary ids or relative paths.

Worktree creation is implemented as a typed Git write family, not a raw
`git worktree add` wrapper. The preview contract is documented in
`docs/internal/plans/2026-06-07-c6-0-worktree-create-preview-contract.md`.
`preview worktree-create` is read-only, does not use `--force` or
`--guess-remote`, does not imply remote branch tracking, and hard-blocks a
branch that is already checked out in another worktree. Target paths are
resolved from config during preview and frozen into `super-git.plan.v0.2`;
execute must not re-expand config templates as trusted authority. `execute`
supports executable `worktree_create` plans only after revalidating the plan
hash, source ref, repository family, branch occupancy, target path, and
post-create HEAD/ref state. Reference commands remain documentation-only.

Worktree create undo is intentionally narrow: remove the clean linked worktree
created by `super-git` when local provenance and state checks still match. It
does not delete branch refs, remote refs, commits, or user-created files.

## Destructive Preview Boundary

`worktree_remove` preview is the first destructive preview boundary.

The guiding rule is:

```text
Worktree remove is not an undoable action; it is a previewable destructive action.
```

This means removal must not copy the `worktree_create` undo model. Removing an
existing linked worktree deletes a filesystem tree and Git worktree metadata.
`super-git` cannot promise to recreate untracked files, ignored files, local
build outputs, editor state, or process state after deletion.

The first `worktree_remove` slice established the preview boundary:

- exact absolute linked-worktree path only
- read-only target scan
- no `--current`
- no `--force`
- no branch, remote-ref, commit, or history deletion
- hard blocks for main, bare-primary, current, detached, staged, unstaged,
  untracked, ignored, conflicted, locked, prunable, in-progress, and submodule
  targets
- running editors, terminals, development servers, and file watchers are not
  detected and must be reported as a limitation
- `reference_commands` stay documentation-only
- `undo_strategy.kind` is `not_available`
- `recovery_hints` are advisory, not a reversibility guarantee

Worktree removal execute requires a separate `super-git.confirmation.v0.1`
artifact. That artifact is explicit authorization, not execution permission by
itself: execute still re-parses and re-hashes the plan, matches the confirmation
to the plan and target identity, revalidates the full target state immediately
before deletion, writes an intent record, then calls `git worktree remove`
without `--force`.

Successful worktree removal results do not return an `undo_token`. The local
execution record states `automatic_undo_available: false` so downstream agents
cannot accidentally treat the destructive action as reversible.

`worktree_remove` is not the only confirmation-gated action: a `history_edit`
plan over a published range also requires a `super-git.confirmation.v0.1`
artifact. The difference is reversibility — a confirmed history edit still
returns an undo token (restore the branch tip), while a confirmed worktree
removal never does.

The confirmation contract is documented in
`docs/internal/plans/2026-06-07-c7-c-worktree-remove-confirmation-contract.md`.

## Risk Vocabulary

The project is converging on a two-axis risk model:

- severity: how much damage a wrong action can cause
- reversibility: how confidently the action can be undone

The current implementation already separates read-only inspect data, preview
plans, guarded execute, and registry-backed undo. `history_edit` exercises both
axes: an unpublished range is medium severity and `reversible_if_unchanged`
with no human confirmation, while a published range is high severity, still
`reversible_if_unchanged` locally, and requires explicit human confirmation
because local undo cannot un-publish remote history. Future work will expand
this into richer warnings and human-confirmation rules for high-risk actions.

## Untrusted Repositories

`super-git` wraps the system `git`, so it inherits Git's standard code-execution
behavior when pointed at a repository whose configuration or hooks are
attacker-controlled. The ambient-environment vectors (`GIT_CONFIG_*`,
`GIT_NAMESPACE`, object-directory and external-diff variables) are scrubbed
before every `git` invocation, but a repository's own `.git/config` and hooks
are part of the repository under inspection.

- **Read commands** (`inspect [path]`, `status [path]`, `wt list [path]`,
  `repo add/save <path>`) accept arbitrary paths. They run `git` with
  `core.fsmonitor` disabled (`-c core.fsmonitor=false`), so a hostile repo's
  `core.fsmonitor` command does not run on read-only inspection. Other
  read-triggered hooks are not a concern for the porcelain commands used here.
- **Write commands** run against the repository the plan is bound to, not an
  arbitrary path. They keep standard Git behavior: in particular,
  `worktree add` fires the repository's `post-checkout` hook. Running a write
  action implies you trust that repository.

Treat inspecting or registering a repository you do not control as carrying the
same risk as running `git status` / `git worktree add` in it yourself.
