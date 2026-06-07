# C7-C Worktree Remove Confirmation Contract

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

> **Status:** C7-F implemented the confirmation-gated execute path described in
> this contract. `worktree_remove` now executes only through
> `super-git execute --plan <file|-> --confirmation <file|->`.

**Goal:** Define the machine-readable confirmation record used by worktree
removal execute support.

**Architecture:** C7-C introduced the confirmation contract before delete
behavior existed. Current execute support validates a separate confirmation
artifact in addition to the preview plan and fresh target state. Confirmation is
explicit authorization, not a substitute for revalidation.

**Tech Stack:** Rust workspace, JSON envelopes, serde-compatible schema design,
existing `super-git.plan.v0.3` destructive preview plans, system `git`
revalidation in execute slices.

---

## Original C7-C Scope

C7-C is a docs-only contract checkpoint. It does not add:

- `worktree_remove` execute support
- `git worktree remove`
- branch deletion
- `--force`
- automatic undo
- confirmation generation commands

C7-C only defined the confirmation model that a later execute slice had to use.
C7-F later implemented that confirmation-gated execute path.

## Decision

Worktree removal execute requires a separate confirmation artifact:

```text
super-git.confirmation.v0.1
```

The confirmation artifact is separate from the plan. A plan's
`confirmation.human_prompt` is display text only; changing that prompt must
never grant execution permission.

The command surface is:

```bash
super-git execute --plan <file|-> --confirmation <file|->
```

The separation is not optional: destructive-action confirmation is not embedded
inside the plan, and `reference_commands` remain documentation-only.

## Confirmation JSON

The first confirmation schema should be:

```json
{
  "schema_version": "super-git.confirmation.v0.1",
  "kind": "destructive_action_confirmation",
  "action": "worktree_remove",
  "plan_schema_version": "super-git.plan.v0.3",
  "plan_id": "sha256:<plan-id>",
  "target": {
    "worktree_list_path": "/abs/repo.worktrees/repo__feature",
    "git_common_dir": "/abs/repo/.git",
    "head": "abc123",
    "branch": "feature"
  },
  "acknowledged_reason_codes": [
    "deletes_worktree_directory",
    "git_worktree_metadata_changes",
    "no_automatic_undo"
  ],
  "acknowledged_undo_strategy": "not_available",
  "acknowledgement": {
    "method": "cli_typed_phrase",
    "phrase": "remove worktree /abs/repo.worktrees/repo__feature without automatic undo"
  }
}
```

The artifact intentionally repeats target identity. This makes accidental or
stale confirmation easier to reject before any fresh Git scan is considered.

## Required Static Validation

Execute support must reject the confirmation before touching Git when any of
these are true:

| Code | Rule |
| --- | --- |
| `confirmation_required` | `worktree_remove` execute is attempted without a confirmation artifact. |
| `confirmation_schema_unsupported` | `schema_version` is not `super-git.confirmation.v0.1`. |
| `confirmation_kind_unsupported` | `kind` is not `destructive_action_confirmation`. |
| `confirmation_action_mismatch` | `action` does not match the plan action. |
| `confirmation_plan_mismatch` | `plan_id` or `plan_schema_version` does not match the plan. |
| `confirmation_target_mismatch` | repeated target identity does not match the plan target. |
| `confirmation_reason_codes_mismatch` | acknowledged reason codes do not exactly match the plan's required reason codes. |
| `confirmation_undo_strategy_mismatch` | acknowledged undo strategy is not `not_available`. |
| `confirmation_acknowledgement_missing` | no explicit acknowledgement method is present. |
| `confirmation_phrase_mismatch` | for CLI typed confirmation, phrase does not match the deterministic phrase derived from the plan target. |

The deterministic CLI phrase is:

```text
remove worktree <target.worktree_list_path> without automatic undo
```

This phrase is intentionally specific and boring. It is not meant to be a
security boundary against a malicious process. It is an explicit friction point
for humans and a machine-checkable contract for agents and UIs.

## Required Fresh Revalidation

A valid confirmation artifact is not enough to delete anything.

Execute removes a worktree only after:

1. Re-parse and re-hash the plan.
2. Confirm action kind is allowlisted.
3. Parse and statically validate the separate confirmation artifact.
4. Match confirmation to plan id, plan schema, action, and target identity.
5. Re-scan the target worktree.
6. Confirm repository family identity still matches.
7. Confirm the target is still the same linked worktree.
8. Confirm the target is not current, main, bare, detached, locked, or prunable.
9. Confirm no in-progress Git operation exists.
10. Confirm staged, unstaged, untracked, ignored, and conflicted counts are zero.
11. Confirm no submodules are present.
12. Write an execution-intent record before calling Git.
13. Rebuild `git worktree remove <target>` from typed fields only.
14. Call Git without shell interpretation and without `--force`.
15. Post-verify that the target is gone from `git worktree list --porcelain`.
16. Record that no automatic undo token is available.

## What Confirmation Is Not

The confirmation artifact is not:

- proof that a human physically typed the phrase
- permission to skip revalidation
- permission to execute `reference_commands`
- permission to delete branches, remote refs, commits, or history
- an undo token
- an archive or backup

This is why user-facing tools should present it as authorization, not
reversibility.

## Implementation Slices

### C7-D: Parse And Reject Confirmation-Gated Remove Plans

C7-D taught `execute` to parse `super-git.plan.v0.3` and reject
`worktree_remove` plans with `confirmation_required` before any delete behavior
existed.

Acceptance:

- [x] no Git writes
- [x] plan hash is recomputed before rejection
- [x] embedded or forged prompt text is ignored
- [x] tests prove a valid-looking remove plan still cannot delete anything

### C7-E: Confirmation Artifact Parsing

C7-E added typed confirmation parsing and static validation, but still did not
delete.

Acceptance:

- [x] invalid confirmation artifacts fail with structured errors
- [x] valid confirmation plus valid plan reached a no-write rejection in C7-E
  before C7-F enabled destructive execute
- [x] confirmation mismatch cases are tested

### C7-F: Worktree Remove Execute

C7-F implemented the destructive write only after C7-D and C7-E were green.

Acceptance:

- all static confirmation checks pass
- all fresh target checks pass
- execution-intent record exists before Git is called
- Git is called as argv, without shell and without `--force`
- branch refs, remote refs, commits, and history remain untouched
- no automatic undo token is returned

## C7-C Self-review Checklist

- [x] Confirmation is explicit and machine-readable.
- [x] Confirmation is separate from display prompt text.
- [x] Confirmation does not grant execution by itself.
- [x] Fresh target revalidation remains mandatory.
- [x] No delete behavior is introduced in this checkpoint.
