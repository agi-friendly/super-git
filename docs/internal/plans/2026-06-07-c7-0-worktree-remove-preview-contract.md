# C7-0 Worktree Remove Preview Contract

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define the safe worktree removal preview contract before any
destructive remove behavior is implemented.

**Architecture:** `worktree_remove` extends the existing preview/execute safety
model, but it must not inherit the `worktree_create` undo mental model.
Removing a worktree deletes a directory and Git worktree metadata. That can be
safe to preview, but it is not automatically undoable.

**Tech Stack:** Rust workspace, serde JSON contracts, clap CLI, system `git`,
`git worktree list --porcelain`, `git rev-parse`, `git status --porcelain=v1
--ignored --untracked-files=all`, worktree-family identity checks, and
integration tests with temporary Git repositories and linked worktrees.

---

## C7-0 Scope

C7-0 is a docs-only contract checkpoint. It does not add
`preview worktree-remove`, `execute`, or `undo` behavior.

The design anchor is:

```text
Worktree remove is not an undoable action; it is a previewable destructive action.
```

The product shape therefore starts as:

```text
inspect -> preview worktree-remove
```

Execution support is deliberately deferred. The first implementation should
produce a plan that tells humans and agents whether removal looks safe, what
would be deleted, why execution is unavailable today, and what manual recovery
would look like if a human later chooses to delete the worktree.

## Non-negotiable Rules

The first worktree remove preview implementation must follow these rules:

- Preview is read-only.
- First scope accepts an exact `--worktree <absolute-linked-worktree-path>`.
- No `--current` shortcut in the first implementation.
- No `--force` behavior.
- No branch deletion.
- No remote-ref deletion.
- No commit/history deletion.
- No automatic undo.
- No shell hooks.
- No copy or archive behavior.
- `reference_commands` are documentation only.
- `execute` must not run a remove plan until a later slice defines explicit
  human confirmation and revalidation.
- Running editors, terminals, development servers, file watchers, and other
  processes inside the target directory are not detected in the first
  implementation. Preview must warn about this limitation.

## Command Surface

Initial preview command:

```bash
super-git preview worktree-remove --worktree <absolute-linked-worktree-path>
```

`--worktree` is intentionally explicit. A path typed by the user or agent is the
object under review. It may be the current worktree in a future UX, but C7 keeps
that out of the first safety surface because `--current` is too easy to invoke
from the wrong terminal.

The preview command fails with `{ ok: false, error }` only when it cannot form a
target-specific plan at all, for example:

- the path is empty
- the path is relative
- the path cannot be canonicalized enough to compare safely
- Git cannot locate a worktree family for the path
- `git worktree list --porcelain` cannot be read
- the path does not exactly match one worktree-list entry

When a family and target can be identified, Git-state problems should normally
produce `{ ok: true, data.execution.status: "blocked" }` so the caller receives
machine-readable reasons.

## Target Identity

Path strings are not enough. A remove preview must identify the target from Git
worktree metadata and Git directory identity.

Minimum target identity fields:

```json
{
  "input_path": "/abs/repo.worktrees/repo__feature",
  "canonical_path": "/abs/repo.worktrees/repo__feature",
  "worktree_list_path": "/abs/repo.worktrees/repo__feature",
  "kind": "linked",
  "worktree_git_dir": "/abs/repo/.git/worktrees/repo__feature",
  "git_common_dir": "/abs/repo/.git",
  "family_id": "sha256:<git-common-dir-identity>"
}
```

Preview must confirm that the target path corresponds to exactly one
`git worktree list --porcelain` entry in the expected family. The main worktree
is not removable. A bare primary repository is not removable. A linked worktree
is the only supported target kind.

The implementation should prefer Git porcelain and Git directory metadata over
human-formatted output. Symlink, casing, and path normalization behavior must be
tested on the local platform, and the plan should record the normalized paths it
used for comparison.

## Required Target Scan

The target scanner must gather enough information to decide whether removal is
safe to preview:

- target worktree kind
- current-process working directory relationship
- lock state
- prunable state
- HEAD commit and branch/detached state
- in-progress operation
- staged paths
- unstaged paths
- untracked paths
- ignored paths
- conflicted paths
- submodule presence
- family membership

Use Git porcelain where possible:

```bash
git -C <target> status --porcelain=v1 --ignored --untracked-files=all
git -C <target> rev-parse --absolute-git-dir
git -C <target> rev-parse --path-format=absolute --git-common-dir
git -C <target> worktree list --porcelain
```

The scanner must not delete, lock, prune, move, or repair anything.

## Hard Blocks

The first remove preview must block removal when any of these are true:

| Code | Reason |
| --- | --- |
| `target_is_main_worktree` | Main worktrees are not removable by this workflow. |
| `target_is_bare_primary` | Bare primary repositories are not removable by this workflow. |
| `target_is_current_worktree` | Avoid deleting the worktree from which the command is running. |
| `target_not_linked_worktree` | Only linked worktrees are in scope. |
| `target_family_mismatch` | The target path and Git metadata do not agree on the family. |
| `target_locked` | Git says the worktree is locked. |
| `target_prunable` | Prunable entries need repair/prune handling, not removal preview. |
| `target_detached` | Detached worktrees are excluded until the recovery story is clearer. |
| `operation_in_progress` | Merge, rebase, apply, cherry-pick, revert, bisect, or similar state is active. |
| `target_has_conflicts` | Conflicted paths require human resolution first. |
| `target_has_staged_changes` | Staged changes would be deleted. |
| `target_has_unstaged_changes` | Unstaged changes would be deleted. |
| `target_has_untracked_files` | Untracked files would be deleted. |
| `target_has_ignored_files` | Ignored files may include local env/build artifacts and must not be silently deleted. |
| `target_has_submodules` | Git worktree remove has incomplete submodule support and may need manual handling. |

These blocks are intentionally stricter than raw Git. `git worktree remove` can
remove some targets with `--force`; `super-git` must not expose that in the
first remove preview.

## Execution Status

C7 preview-only plans should not advertise immediate execution.

Recommended values:

```json
{
  "execution": {
    "status": "preview_only",
    "execute_supported": false,
    "future_execute_eligibility": "needs_human_confirmation",
    "raw_git_allowed": false,
    "blocked_reasons": []
  }
}
```

If hard blocks exist, the status becomes:

```json
{
  "execution": {
    "status": "blocked",
    "execute_supported": false,
    "future_execute_eligibility": "blocked",
    "raw_git_allowed": false,
    "blocked_reasons": [
      {
        "code": "target_has_untracked_files",
        "severity": "hard_block"
      }
    ]
  }
}
```

`preview_only` means: the target appears removable under the current strict
policy, but `super-git execute` must still refuse the plan until a later C7/C8
slice defines the confirmation and revalidation contract.

## Risk And Confirmation

Worktree removal is high-severity because the visible effect is deletion of a
filesystem tree.

Minimum risk fields:

```json
{
  "risk": {
    "severity": "high",
    "reversibility": "not_automatically_reversible",
    "requires_human_confirmation": true
  },
  "confirmation": {
    "required_before_execute": true,
    "reason_codes": [
      "deletes_worktree_directory",
      "git_worktree_metadata_changes",
      "no_automatic_undo"
    ],
    "human_prompt": "Remove linked worktree at /abs/repo.worktrees/repo__feature?"
  }
}
```

`requires_human_confirmation` stays in `risk` for continuity with existing plan
metadata. The `confirmation` object carries the action-specific details a future
execute flow will need.

## Undo And Recovery

`worktree_remove` must not claim undo support.

Use this shape:

```json
{
  "undo_strategy": {
    "kind": "not_available",
    "reason": "Removing an existing worktree deletes a filesystem tree and Git worktree metadata. super-git cannot recreate untracked or ignored files."
  },
  "recovery_hints": [
    {
      "kind": "recreate_worktree",
      "description": "If the branch still exists, a human may recreate a linked worktree from the branch.",
      "reference_command": ["git", "worktree", "add", "/abs/new-path", "feature"]
    }
  ]
}
```

Recovery hints are documentation, not a reversible guarantee. They must not be
used by `execute` as commands.

## Plan Schema

C7-B uses `super-git.plan.v0.3` because destructive preview introduces a common
preview-side confirmation/recovery contract that future destructive actions can
reuse.

That version bump is justified by these common fields:

- `execution.execute_supported`
- `execution.future_execute_eligibility`
- `confirmation`
- `undo_strategy.kind: "not_available"`
- `recovery_hints`

Preview output remains wrapped by the global JSON envelope:

```json
{
  "ok": true,
  "data": {
    "schema_version": "super-git.plan.v0.3",
    "plan_id": "sha256:<canonical-plan-hash>",
    "action": {
      "kind": "worktree_remove",
      "options": {
        "worktree": "/abs/repo.worktrees/repo__feature"
      }
    },
    "repository": {
      "family_id": "sha256:<git-common-dir-identity>",
      "git_common_dir": "/abs/repo/.git",
      "main_worktree": "/abs/repo",
      "selected_from": "/abs/repo.worktrees/repo__feature"
    },
    "target": {
      "input_path": "/abs/repo.worktrees/repo__feature",
      "canonical_path": "/abs/repo.worktrees/repo__feature",
      "worktree_list_path": "/abs/repo.worktrees/repo__feature",
      "kind": "linked",
      "worktree_git_dir": "/abs/repo/.git/worktrees/repo__feature",
      "git_common_dir": "/abs/repo/.git",
      "head": "abc123",
      "branch": "refs/heads/feature",
      "detached": false,
      "locked": false,
      "prunable": false,
      "is_current_worktree": false,
      "has_submodules": false
    },
    "target_state": {
      "operation": "none",
      "working_tree": {
        "clean": true,
        "staged": 0,
        "unstaged": 0,
        "untracked": 0,
        "ignored": 0,
        "conflict_count": 0,
        "conflicts": []
      }
    },
    "preconditions": [
      { "code": "target_is_linked_worktree", "status": "passed" },
      { "code": "target_not_current_worktree", "status": "passed" },
      { "code": "target_clean_including_ignored", "status": "passed" },
      { "code": "target_has_no_submodules", "status": "passed" }
    ],
    "execution": {
      "status": "preview_only",
      "execute_supported": false,
      "future_execute_eligibility": "needs_human_confirmation",
      "raw_git_allowed": false,
      "suggested_super_git_command": null,
      "blocked_reasons": []
    },
    "risk": {
      "severity": "high",
      "reversibility": "not_automatically_reversible",
      "requires_human_confirmation": true
    },
    "confirmation": {
      "required_before_execute": true,
      "reason_codes": [
        "deletes_worktree_directory",
        "git_worktree_metadata_changes",
        "no_automatic_undo"
      ],
      "human_prompt": "Remove linked worktree at /abs/repo.worktrees/repo__feature?"
    },
    "effects": [
      "Delete the linked worktree directory at /abs/repo.worktrees/repo__feature.",
      "Remove the linked worktree entry from Git worktree metadata.",
      "Preserve branch refs, remote refs, commits, and history."
    ],
    "limitations": [
      "Preview cannot detect editors, terminals, development servers, or file watchers using the target path."
    ],
    "reference_commands": {
      "semantics": "documentation_only",
      "never_execute_directly": true,
      "commands": [
        ["git", "worktree", "remove", "/abs/repo.worktrees/repo__feature"]
      ]
    },
    "undo_strategy": {
      "kind": "not_available",
      "reason": "Existing worktree removal is destructive and cannot restore untracked or ignored files."
    },
    "recovery_hints": [
      {
        "kind": "recreate_worktree",
        "description": "If the branch still exists, a human may recreate a linked worktree from the branch.",
        "reference_command": ["git", "worktree", "add", "/abs/new-path", "feature"]
      }
    ]
  }
}
```

## Plan Hash Inputs

`plan_id` must bind the fields that make future execution safe:

- schema version
- action kind and options
- repository family identity
- target identity
- target HEAD and branch/detached state
- target lock/prunable state
- target working-tree summary including ignored files
- target operation state
- submodule detection result
- preconditions
- execution status
- blocked reason codes and details
- risk
- confirmation requirements
- undo strategy
- recovery-hint kinds

`plan_id` must exclude advisory prose:

- effects
- limitations
- reference commands
- human prompts
- recovery-hint descriptions
- timestamps

## Future Execute Contract

This is not part of C7-0 or the first preview implementation, but future execute
support must satisfy all of these before it can remove anything:

1. Parse and validate the plan schema.
2. Recompute the plan hash.
3. Confirm action kind is allowlisted.
4. Confirm `execution.status` is eligible for execute.
5. Require explicit human confirmation.
6. Confirm repository family identity still matches.
7. Confirm target identity still matches the previewed worktree.
8. Confirm target is still linked, unlocked, non-prunable, and not current.
9. Confirm target is not detached.
10. Confirm target has no in-progress Git operation.
11. Confirm target has no staged, unstaged, untracked, ignored, or conflicted
    paths.
12. Confirm target has no submodules.
13. Write an execution-intent record before calling Git.
14. Rebuild the trusted `git worktree remove` arguments from typed fields.
15. Run Git without shell interpretation and without `--force`.
16. Post-verify that `git worktree list --porcelain` no longer reports the
    target.
17. Record that no automatic undo token is available.

## Implementation Slices After C7-0

### C7-A: Target resolver and scanner

**Files:**

- Modify or create: `crates/super-git-core/src/git/worktree_remove.rs`
- Modify: `crates/super-git-core/src/git/worktree.rs`
- Modify: `crates/super-git-core/src/git/mod.rs`
- Test: core unit tests and integration helpers for removable and blocked
  targets

Acceptance:

- Exact absolute target path is required.
- Main worktree, bare primary, current worktree, detached worktree, locked,
  prunable, staged, unstaged, untracked, ignored, conflicted, in-progress, and
  submodule targets are detected.
- The scanner performs no writes.
- Target identity is based on Git worktree metadata, not only path strings.

### C7-B: `preview worktree-remove`

**Files:**

- Modify or create: `crates/super-git-core/src/git/preview_worktree_remove.rs`
- Modify: `crates/super-git-core/src/model.rs`
- Modify: `crates/super-git-cli/src/args.rs`
- Modify: `crates/super-git-cli/src/main.rs`
- Modify: `crates/super-git-cli/src/output.rs`
- Test: `crates/super-git-cli/tests/preview_worktree_remove.rs`

Acceptance:

- `preview worktree-remove --worktree <abs-path>` emits a structured plan.
- Clean linked worktrees produce `execution.status: "preview_only"` and
  `execute_supported: true` after C7-F added confirmed remove execute support.
- Blocked targets produce `execution.status: "blocked"` and structured
  blocked reasons.
- Plans include risk, confirmation, undo-unavailable, and recovery-hint fields.
- Preview performs no writes.

### C7-C: Confirmation model checkpoint

Before `execute` can support worktree removal, define the execute-side human
confirmation record. The preview-side confirmation/recovery fields are already
part of `super-git.plan.v0.3`, but execute still needs an explicit,
machine-readable acknowledgement that cannot be forged by changing prompt text.

The C7-C checkpoint records that contract in
`docs/internal/plans/2026-06-07-c7-c-worktree-remove-confirmation-contract.md`.

Acceptance:

- Confirmation is explicit and machine-readable.
- `execute` cannot be tricked by a forged prompt string.
- The confirmation record is not enough by itself; execute still revalidates
  all target state immediately before deletion.

## Deferred Features

These are product ideas, not part of the first worktree remove preview:

- `--current`
- `--force`
- deleting branches after worktree removal
- cleaning prunable metadata
- repairing broken worktree entries
- archiving target files before deletion
- process detection for running editors or dev servers
- batch worktree removal
- dashboard-driven removal
- automatic recreation after removal

## C7-0 Self-review Checklist

- [x] The contract does not allow preview to write.
- [x] The contract does not claim automatic undo.
- [x] The contract blocks staged, unstaged, untracked, ignored, conflicted,
      locked, prunable, detached, current, main, bare, and submodule targets.
- [x] The contract keeps `reference_commands` documentation-only.
- [x] The contract records the limitation around running processes.
- [x] The contract keeps future execute behind explicit human confirmation.
- [x] The contract distinguishes destructive preview from create undo.
