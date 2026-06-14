# C8-0 History Edit Preview Contract

> **Status:** Active contract checkpoint for Stage 6 plan-based history edit.
> No `history_edit` preview, execute, or undo behavior exists yet.

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define the plan-based history edit contract before any history
rewrite behavior is implemented.

**Architecture:** `history_edit` extends the preview/execute/undo lifecycle to
commit-message and commit-boundary edits on the current branch. The first op
set (`pick`, `reword`, `squash`, `fixup`) never changes any tree, so execute
rebuilds the commit chain with Git plumbing and moves the branch ref with a
compare-and-swap update. Interactive rebase machinery is never invoked, not
even internally.

**Tech Stack:** Rust workspace, serde JSON contracts, clap CLI, system `git`,
`git rev-list`, `git cat-file`, `git commit-tree`, `git update-ref` with
old-value checks, SHA-256 plan fingerprints, integration tests with temporary
Git repositories.

---

## C8-0 Scope

C8-0 is a docs-only contract checkpoint. It does not add
`preview history-edit`, `execute`, or `undo` behavior.

The design anchor is:

```text
For the first op set, history edit is a ref-and-object operation,
not a working-tree operation.
```

`git rebase -i` depends on an interactive editor that most agent harnesses
cannot open. Agents fall back to fragile `GIT_SEQUENCE_EDITOR` scripting, and
that is a top history-loss failure mode for weaker models. The safe product
shape is therefore:

```text
inspect -> preview history-edit -> execute --plan -> undo --token
```

For ranges that contain published commits, execute additionally requires the
destructive confirmation artifact from the C7-C contract family:

```text
inspect -> preview history-edit -> execute --plan --confirmation
```

## Why The First Op Set Cannot Conflict

`pick`, `reword`, `squash`, and `fixup` keep every original tree object:

- `pick` and `reword` reuse the original commit's tree unchanged.
- `squash` and `fixup` fold a commit into its predecessor; the folded result
  reuses the tree of the last original commit in the fold chain.
- No patch is ever re-applied, so there is nothing that can conflict.

This yields a hard post-verification invariant:

```text
tree(new branch tip) == tree(old branch tip)
```

Execute must verify this invariant after rewriting. If it does not hold, the
implementation has a bug and execute must roll the branch ref back.

Ops that re-apply patches (`drop`, reorder, `split`, `edit`) can conflict and
are excluded until Stage 7 conflict prediction exists. This includes
path-disjoint `drop` cases that look safe: any drop changes the final tree
and silently reverts content without a conflict signal, which is exactly the
class of surprise this op set excludes by construction.

> **Status update (2026-06-14):** Stage 7 prediction exists. `drop` has its
> own checkpoint (`2026-06-12-c8-drop-history-edit-contract.md`) and is now
> implemented through preview, execute, and undo. `reorder` has its own
> checkpoint (`2026-06-13-c8-reorder-history-edit-contract.md`) and is now
> implemented as a prediction-gated, tree-preserving, ref-only history edit.
> The tree-preserving invariant in this document remains the baseline for
> `pick`/`reword`/`squash`/`fixup` and clean reorder plans; `drop` is the
> explicit tree-changing exception.

## Non-negotiable Rules

The first history edit implementation must follow these rules:

- Preview is read-only.
- Execute never runs `git rebase`, never spawns an editor, and never sets
  `GIT_SEQUENCE_EDITOR`. New commits are built with plumbing commands from
  typed plan data.
- Working-tree files and the index are never modified by execute or undo.
- The final tree of the branch tip must be identical before and after execute,
  and execute must post-verify that invariant.
- Branch ref updates must use compare-and-swap (`git update-ref` with the
  expected old value). A lost race fails cleanly instead of clobbering.
- Author name, email, and date are preserved on every rewritten commit. Only
  messages and commit boundaries change. Committer identity and timestamp
  follow normal Git rewrite semantics (current identity, current time).
- No branch deletion, no remote-ref changes, no network access, no fetch, no
  push, no autostash, no `--force` anything.
- Instructions must enumerate every commit in the range exactly once, in the
  original order. Incomplete or stale instruction lists are blocked, not
  repaired silently.
- Rewriting commits reachable from any local remote-tracking ref requires the
  separate confirmation artifact; it must never execute silently.
- `reference_commands` are documentation only.
- Undo restores the branch pointer only and must validate local provenance
  first.

## Command Surface

Initial preview command:

```bash
super-git preview history-edit --base <ref> [--instructions <file|->]
```

`--base` names the last commit that stays untouched. The editable range is
`base..HEAD` on the branch checked out in the current worktree.

Without `--instructions`, preview returns a read-only survey plan: the full
range with per-commit identity, full messages, published and signed flags,
and the same hard-block analysis, but `execution.status: "survey"` and
`execute_supported: false`. The survey is the intended entry point for
agents: its `range.commits` array is the exact template the instruction list
must follow, so an agent never reconstructs history from `git log` parsing by
hand. Published detection in particular is not something an agent should
recompute itself.

With `--instructions`, preview validates the declarative instruction list
against the real repository state and produces an executable, blocked, or
confirmation-gated plan.

The preview command fails with `{ ok: false, error }` only when it cannot form
a target-specific plan at all, for example:

- the current directory is not inside a Git worktree
- `--base` cannot be resolved to a commit
- the instructions input is provided but unreadable or not valid JSON for
  `super-git.instructions.v0.1`

When the branch, base, and instructions can be identified, Git-state and
instruction-content problems should produce
`{ ok: true, data.execution.status: "blocked" }` with machine-readable
reasons, so weaker agents can read the blocked plan, fix their instruction
list, and retry without guessing.

## Range Policy

The first implementation edits the current branch in place:

- HEAD must be attached to a local branch; detached HEAD is blocked.
- The resolved base commit must be an ancestor of HEAD. Diverged bases imply
  "rebase onto", which changes parentage and belongs to the Stage 7 family.
- The range `base..HEAD` must be non-empty and contain no merge commits. With
  merges excluded, an ancestor base guarantees a linear chain.
- The range is capped at 100 commits in the first implementation. The cap is a
  guardrail against a wrong `--base` (for example, pointing at the root commit
  by accident), and a blocked plan reports the actual count.
- Editing the root commit is out of scope because `--base` must name an
  existing ancestor commit.

## Instruction Set

Instructions are provided as a small versioned JSON document:

```json
{
  "schema_version": "super-git.instructions.v0.1",
  "action": "history_edit",
  "base": "main",
  "items": [
    { "commit": "aaa111", "op": "pick" },
    {
      "commit": "bbb222",
      "op": "reword",
      "message": "feat(login): validate email before submit"
    },
    { "commit": "ccc333", "op": "fixup" }
  ]
}
```

Input rules:

- `items` must cover every commit in `base..HEAD` exactly once, oldest first.
- `commit` may be abbreviated but must resolve unambiguously to a commit
  inside the range. The plan freezes full object ids.
- `reword` requires `message`. The message replaces the original completely.
- `squash` folds the commit into its nearest preceding non-fold item and
  requires `message`, which becomes the combined commit's full message. There
  is no editor to compose a merged message, so the contract demands it
  explicitly instead of concatenating silently. Survey and plan output carry
  every range commit's full message, so the agent composes the combined
  message from real context instead of guessing.
- `fixup` folds like `squash` but keeps the preceding item's message and
  discards its own.
- When a fold group carries several message-bearing ops, the last one wins;
  the result stays deterministic without an editor to merge messages.
- `pick` and `fixup` must not carry `message`. A message there would be
  silently ignored, and silent intent loss is worse for agents than a block.
- The first item must be `pick` or `reword`; a fold needs a predecessor.
- Messages must be non-empty after trimming and are normalized to end with
  exactly one trailing newline; otherwise they are stored verbatim.
- An all-`pick` list with no rewords and no folds is blocked as a meaningless
  write.

Op support in the first implementation:

| Op | Preview | Execute |
| --- | --- | --- |
| `pick` | supported | supported |
| `reword` | supported | supported |
| `squash` | supported | supported |
| `fixup` | supported | supported |
| `drop` | supported via C8-drop checkpoint | supported with confirmation + worktree sync |
| reorder (list order change) | supported via C8-reorder checkpoint | supported when prediction preserves final tree |
| `edit` / `split` | blocked | deferred |

## Published Commit Policy

A range commit is treated as published when it is reachable from any local
remote-tracking ref (`refs/remotes/*`). The scan basis is recorded in the
plan, computed from porcelain-safe plumbing such as `git rev-list` with
remote-tracking negation.

Honest limitation: the scan only sees refs from the last fetch. `super-git`
never fetches on its own, so a commit pushed from elsewhere can look
unpublished locally. The plan must record this limitation, and the docs must
recommend fetching before editing shared branches.

Policy:

- No published commits in range: the plan is `executable` and execute returns
  an undo token.
- Any published commit in range: the plan is `preview_only` and execute
  requires a `super-git.confirmation.v0.1` artifact, because collaborators'
  clones diverge and local undo cannot un-publish anything.

## Hard Blocks

| Code | Reason |
| --- | --- |
| `head_detached` | History edit operates on the branch checked out in the current worktree. |
| `operation_in_progress` | Merge, rebase, apply, cherry-pick, revert, bisect, or similar state is active. |
| `conflicts_present` | Conflicted index entries indicate unresolved history state even when no operation is detected. |
| `base_not_ancestor_of_head` | Diverged bases imply re-parenting, which is Stage 7 territory. |
| `range_empty` | Base equals HEAD; nothing to edit. |
| `range_too_large` | More than 100 commits in range; likely a wrong `--base`. |
| `merge_commit_in_range` | Merge topology preservation is out of scope. |
| `commit_signing_enabled` | `commit.gpgsign` is set; rebuilt commits cannot honestly honor it without interactive key access. Deferred. |
| `committer_identity_missing` | `user.name` or `user.email` is not configured. |
| `instruction_op_unsupported` | `edit`, `split`, or unknown ops. `drop` moved to the C8-drop checkpoint; reorder moved to the C8-reorder checkpoint. |
| `instructions_incomplete` | One or more range commits are missing from the list. |
| `instructions_unknown_commit` | An item references a commit outside the range. |
| `instructions_duplicate_commit` | A commit appears more than once. |
| `instructions_order_mismatch` | Baseline C8-0 order guard for non-reorder instruction mistakes. Current reorder plans are handled by the C8-reorder checkpoint and may instead be accepted or blocked by reorder-specific reasons. |
| `instruction_fold_without_predecessor` | The first item is `squash` or `fixup`. |
| `instruction_message_missing` | `reword` or `squash` without a message. |
| `instruction_message_empty` | A message is empty after trimming. |
| `instruction_message_unexpected` | `pick` or `fixup` carries a message that would be silently ignored. |
| `instructions_no_effective_change` | All items are `pick`; the edit would be a no-op. |

Staged, unstaged, untracked, and ignored files are intentionally allowed. C7
blocks dirty state because worktree removal deletes files; history edit is a
ref-and-object operation that never touches the working tree or the index, so
blocking dirty state here would contradict the design anchor and force
stash-edit-unstash detours for no safety gain. The old and new tips share one
tree, so every staged and unstaged diff is byte-identical before and after
execute.

Warnings (not blocks):

- `working_tree_dirty` when staged or unstaged changes exist, so callers see
  that the edit ran against a dirty tree on purpose.
- `signed_commits_lose_signatures` when any range commit carries a signature.
  Rewritten commits do not preserve GPG/SSH signatures from the originals.

## Execution Status In Preview

The v0.4 execution block unifies the v0.2/v0.3 shapes:

```json
{
  "execution": {
    "status": "executable",
    "execute_supported": true,
    "requires_confirmation_artifact": false,
    "raw_git_allowed": false,
    "suggested_super_git_command": ["super-git", "execute", "--plan", "<plan-file>"],
    "blocked_reasons": []
  }
}
```

When the range contains published commits:

```json
{
  "execution": {
    "status": "preview_only",
    "execute_supported": true,
    "requires_confirmation_artifact": true,
    "raw_git_allowed": false,
    "suggested_super_git_command": [
      "super-git", "execute",
      "--plan", "<plan-file>",
      "--confirmation", "<confirmation-file>"
    ],
    "blocked_reasons": []
  }
}
```

A survey preview (no instructions) reports:

```json
{
  "execution": {
    "status": "survey",
    "execute_supported": false,
    "requires_confirmation_artifact": false,
    "raw_git_allowed": false,
    "suggested_super_git_command": [
      "super-git", "preview", "history-edit",
      "--base", "<ref>",
      "--instructions", "<instructions-file>"
    ],
    "blocked_reasons": []
  }
}
```

Survey plans set `instructions` and `result_summary` to `null` and are never
eligible for execute. Hard blocks apply to surveys too, so an agent learns
about a detached HEAD or an oversized range before writing any instructions.

When hard blocks exist, `status` is `blocked`, `execute_supported` is `false`,
and `blocked_reasons` carries structured codes with details (for example,
`instructions_incomplete` lists the missing commit ids), so an agent can
repair its instruction list from the plan alone.

## Risk Tiers

Unpublished range:

```json
{
  "risk": {
    "severity": "medium",
    "reversibility": "reversible_if_unchanged",
    "requires_human_confirmation": false
  }
}
```

Published range:

```json
{
  "risk": {
    "severity": "high",
    "reversibility": "reversible_if_unchanged",
    "requires_human_confirmation": true
  },
  "confirmation": {
    "required_before_execute": true,
    "reason_codes": [
      "rewrites_published_commits",
      "remote_branches_will_diverge",
      "local_undo_does_not_unpublish"
    ],
    "human_prompt": "Rewrite published history on refs/heads/feature/login?"
  }
}
```

Reversibility stays `reversible_if_unchanged` in both tiers because the local
branch pointer can always be restored while the old tip is reachable. The
published tier is high severity because restoring the local pointer does not
repair collaborators' clones or remote refs.

## Plan Schema

History edit needs typed fields for branch identity, range commits, published
analysis, and the instruction list, so it extends the plan family as:

```text
super-git.plan.v0.4
```

Preview output remains wrapped by the global JSON envelope:

```json
{
  "ok": true,
  "data": {
    "schema_version": "super-git.plan.v0.4",
    "plan_id": "sha256:<canonical-plan-hash>",
    "action": {
      "kind": "history_edit",
      "options": {
        "base": "main"
      }
    },
    "repository": {
      "family_id": "sha256:<git-common-dir-identity>",
      "git_common_dir": "/abs/repo/.git",
      "worktree_root": "/abs/repo"
    },
    "branch": {
      "ref": "refs/heads/feature/login",
      "short_name": "feature/login",
      "tip_commit": "ccc333",
      "checked_out_at": "/abs/repo",
      "upstream": "refs/remotes/origin/feature/login"
    },
    "range": {
      "base_input": "main",
      "base_commit": "aaa000",
      "base_is_ancestor_of_head": true,
      "order": "oldest_first",
      "commit_count": 3,
      "commits": [
        {
          "commit": "aaa111",
          "subject": "feat(login): add form",
          "message": "feat(login): add form\n",
          "author_name": "Jane Dev",
          "author_email": "jane@example.com",
          "author_date": "2026-06-09T10:00:00+09:00",
          "published": false,
          "signed": false,
          "is_merge": false
        },
        {
          "commit": "bbb222",
          "subject": "fix typo",
          "message": "fix typo\n",
          "author_name": "Jane Dev",
          "author_email": "jane@example.com",
          "author_date": "2026-06-09T11:00:00+09:00",
          "published": false,
          "signed": false,
          "is_merge": false
        },
        {
          "commit": "ccc333",
          "subject": "wip",
          "message": "wip\n",
          "author_name": "Jane Dev",
          "author_email": "jane@example.com",
          "author_date": "2026-06-09T12:00:00+09:00",
          "published": false,
          "signed": false,
          "is_merge": false
        }
      ]
    },
    "published_scan": {
      "basis": "local_remote_tracking_refs",
      "published_commits": []
    },
    "instructions": {
      "schema_version": "super-git.instructions.v0.1",
      "order": "oldest_first",
      "items": [
        { "commit": "aaa111", "op": "pick" },
        {
          "commit": "bbb222",
          "op": "reword",
          "message": "feat(login): validate email before submit\n"
        },
        { "commit": "ccc333", "op": "fixup" }
      ]
    },
    "result_summary": {
      "commits_before": 3,
      "commits_after": 2,
      "messages_changed": 1,
      "commits_folded": 1,
      "final_tree_unchanged": true
    },
    "preconditions": [
      { "code": "head_attached_to_local_branch", "status": "passed" },
      { "code": "operation_none", "status": "passed" },
      { "code": "no_conflicted_paths", "status": "passed" },
      { "code": "base_is_ancestor_of_head", "status": "passed" },
      { "code": "range_linear_without_merges", "status": "passed" },
      { "code": "commit_signing_disabled", "status": "passed" },
      { "code": "committer_identity_configured", "status": "passed" },
      { "code": "instructions_match_range", "status": "passed" }
    ],
    "execution": {
      "status": "executable",
      "execute_supported": true,
      "requires_confirmation_artifact": false,
      "raw_git_allowed": false,
      "suggested_super_git_command": ["super-git", "execute", "--plan", "<plan-file>"],
      "blocked_reasons": []
    },
    "risk": {
      "severity": "medium",
      "reversibility": "reversible_if_unchanged",
      "requires_human_confirmation": false
    },
    "warnings": [],
    "effects": [
      "Rewrite 3 commits on refs/heads/feature/login into 2 commits.",
      "Change 1 commit message and fold 1 fixup commit.",
      "Preserve every author name, email, and date.",
      "Preserve the final tree; working-tree files and the index do not change."
    ],
    "limitations": [
      "Published detection only sees local remote-tracking refs from the last fetch.",
      "Undo depends on the previous tip staying reachable in the local object store.",
      "Rewritten commits do not preserve GPG/SSH signatures from the originals."
    ],
    "reference_commands": {
      "semantics": "documentation_only",
      "never_execute_directly": true,
      "commands": [
        ["git", "rebase", "-i", "main"]
      ]
    },
    "undo_strategy": {
      "kind": "restore_branch_tip_snapshot",
      "deletes_branch": false,
      "deletes_history": false
    },
    "undo_preview": {
      "kind": "restore_branch_tip_snapshot",
      "available_after_execute": true,
      "limitations": [
        "Undo refuses if the branch tip moved after execute.",
        "Undo requires the previous tip commit to still exist locally (reflog/gc window).",
        "Undo restores the branch pointer only; it never touches working-tree files.",
        "Undo does not un-publish anything that was pushed."
      ]
    }
  }
}
```

## Plan Hash Inputs

`plan_id` must bind the data that makes execution safe:

- schema version
- action kind and options
- repository family identity
- branch ref, tip commit, and checked-out worktree path
- base input and resolved base commit
- range commit object ids in order
- per-commit published and signed flags
- published scan basis
- the full instruction list, including ops, frozen full object ids, and
  messages, or its explicit absence in survey plans
- result summary
- preconditions
- execution status and blocked reason codes with details
- risk
- confirmation requirements when the range is published
- undo strategy

`plan_id` must exclude advisory prose:

- commit subjects, full original messages, and author display fields
  (derived from bound object ids)
- effects
- limitations
- reference commands
- suggested `super-git` command
- human prompts
- timestamps

The v0.4 plan hash reuses the C4 canonical JSON rule: UTF-8 JSON object,
sorted object keys, no insignificant whitespace, and the same representation
in tests and production code. The v0.4 domain separator is:

```text
super-git-plan-v0.4\n
```

## Execute Contract

`execute --plan` for `history_edit` must:

1. Parse and validate the plan schema.
2. Recompute the plan hash.
3. Confirm action kind is allowlisted.
4. Confirm `execution.status` is eligible: `executable`, or `preview_only`
   with a valid confirmation artifact for published ranges. Survey and
   blocked plans are never eligible.
5. For published ranges, statically validate the confirmation artifact under
   the C7-C rules adapted to `history_edit` before touching Git.
6. Confirm repository family identity still matches.
7. Confirm the plan branch is still checked out at the recorded worktree.
8. Confirm the branch tip still equals the plan tip commit.
9. Re-resolve `base..HEAD` and confirm the commit ids and order match the
   plan exactly.
10. Re-run the published scan and confirm it matches the plan. New published
    commits since preview make the plan stale and blocked.
11. Confirm no in-progress operation and no conflicted paths. Staged and
    unstaged changes are allowed; the mechanism never touches them.
12. Confirm `commit.gpgsign` is not enabled and committer identity is
    configured.
13. Write an execution-intent record before any ref write.
14. Rebuild the commit chain bottom-up with plumbing, reusing original tree
    object ids, preserving author fields, and applying messages and folds
    from typed instruction data only. Original commits before the first
    effective change keep their original object ids.
15. Update the branch ref with compare-and-swap: expected old value is the
    plan tip commit. A CAS failure aborts with a structured error and no
    state change; newly written objects are unreachable and harmless.
16. Post-verify: the branch tip equals the new tip, `tree(new tip)` equals
    `tree(old tip)`, and the new commit count matches `result_summary`. The
    working-tree status summary is compared as a sanity signal only; drift
    (for example, an editor saving a file during execute) is reported as a
    warning, not a failure, because those files are not super-git's to
    manage.
17. If post-verification fails, attempt a compare-and-swap rollback to the
    old tip and report `failed_rolled_back`; if the rollback CAS also fails,
    report `failed_partial` with the execution record path and both tip ids.
18. Write a local execution record and return an undo token containing the
    branch ref, old tip, and new tip.

HEAD itself is never rewritten directly; it stays attached to the branch name
while the branch ref moves. Because the old and new tips share one tree, the
index and working tree remain valid without any checkout.

## Confirmation Artifact For Published Ranges

Published-history rewrites reuse the C7-C artifact shape:

```json
{
  "schema_version": "super-git.confirmation.v0.1",
  "kind": "destructive_action_confirmation",
  "action": "history_edit",
  "plan_schema_version": "super-git.plan.v0.4",
  "plan_id": "sha256:<plan-id>",
  "target": {
    "branch_ref": "refs/heads/feature/login",
    "git_common_dir": "/abs/repo/.git",
    "tip_commit": "ccc333"
  },
  "acknowledged_reason_codes": [
    "rewrites_published_commits",
    "remote_branches_will_diverge",
    "local_undo_does_not_unpublish"
  ],
  "acknowledged_undo_strategy": "restore_branch_tip_snapshot",
  "acknowledgement": {
    "method": "cli_typed_phrase",
    "phrase": "rewrite published history on refs/heads/feature/login at ccc333 for plan a1b2c3d4e5f6"
  }
}
```

The deterministic CLI phrase is:

```text
rewrite published history on <branch.ref> at <branch.tip_commit> for plan <short-plan-id>
```

Static validation reuses the C7-C rule table with `history_edit` identity
fields. As in C7, the artifact is authorization, not reversibility, and never
replaces fresh revalidation. Unlike `worktree_remove`, the acknowledged undo
strategy is `restore_branch_tip_snapshot`, and the reason codes spell out that
local undo does not repair remote refs or collaborators' clones.

## Undo Contract

History edit undo means:

```text
move the branch ref back to the recorded pre-execute tip
```

It does not mean deleting branches, deleting commits, editing working-tree
files, or pushing anything.

Undo must validate:

- token schema
- local execution-record provenance under the Git common directory
- the branch ref still exists
- the current branch tip equals the recorded new tip; a moved tip refuses
  with `branch_advanced_since_execute`
- the recorded old tip commit still exists in the local object store
- no in-progress Git operation

Then undo updates the branch ref with compare-and-swap from the new tip back
to the old tip, and post-verifies:

- the branch tip equals the old tip
- working-tree status drift is reported as a warning only
- no refs other than the target branch were modified

The rewritten commits remain in the object store and reflog after undo; they
become unreachable garbage that normal Git maintenance collects later.

## Implementation Slices After C8-0

### C8-A: Range resolver and instruction validation

**Files:**

- Create: `crates/super-git-core/src/git/history_edit.rs`
- Modify: `crates/super-git-core/src/git/mod.rs`
- Test: core unit tests for range resolution, published scan, and
  instruction validation against real temporary repositories

Acceptance:

- [x] Branch, base, ancestor, linearity, cap, and merge checks match this
      contract.
- [x] Published scan is computed from local remote-tracking refs only.
- [x] Instruction parsing enforces completeness, order, fold, and message
      rules with structured reason codes.
- [x] Fold computation produces the expected result summary, including the
      unchanged-prefix optimization boundary.
- [x] The resolver performs no writes.

### C8-B: `preview history-edit`

**Files:**

- Create: `crates/super-git-core/src/git/preview_history_edit.rs`
- Modify: `crates/super-git-core/src/model.rs`
- Modify: `crates/super-git-cli/src/args.rs`
- Modify: `crates/super-git-cli/src/main.rs`
- Modify: `crates/super-git-cli/src/output.rs`
- Test: `crates/super-git-cli/tests/preview_history_edit.rs`

Acceptance:

- [x] `preview history-edit --base <ref> --instructions <file|->` emits a
      `super-git.plan.v0.4` plan with all fields in this contract.
- [x] `preview history-edit --base <ref>` without instructions emits a survey
      plan with full range data, `status: "survey"`,
      `execute_supported: false`, and null instructions/result summary.
- [x] Unpublished ranges with valid instructions report `executable`.
- [x] Published ranges report `preview_only` with
      `requires_confirmation_artifact: true`.
- [x] Every hard block in this contract produces `status: "blocked"` with
      structured reasons and details.
- [x] Plan hash inputs and exclusions match this contract.
- [x] Preview performs no writes.

### C8-C: Execute unpublished history edit plans

**Files:**

- Create: `crates/super-git-core/src/git/execute_history_edit.rs`
- Modify: `crates/super-git-core/src/git/execute.rs`
- Modify: `crates/super-git-core/src/git/undo_registry.rs`
- Test: `crates/super-git-cli/tests/execute_history_edit.rs`

Acceptance:

- [x] Execute revalidates everything in the execute contract before writing.
- [x] Survey and blocked plans are rejected before any write.
- [x] Commits are rebuilt with plumbing; `git rebase` is never invoked.
- [x] The branch ref moves only through compare-and-swap.
- [x] Tree-identity, commit-count, and status-unchanged post-verification is
      enforced, including the rollback path.
- [x] Author fields are preserved on rewritten commits.
- [x] A published range without confirmation is rejected with
      `confirmation_required` before any write.
- [x] Successful execute returns an undo token and writes a local execution
      record.

### C8-D: Undo history edit

**Files:**

- Create: `crates/super-git-core/src/git/undo_history_edit.rs`
- Modify: `crates/super-git-core/src/git/undo.rs`
- Test: `crates/super-git-cli/tests/undo_history_edit.rs`

Acceptance:

- [x] Undo validates provenance, tip identity, old-tip reachability, and
      operation state before moving the ref.
- [x] A branch tip that advanced after execute refuses with
      `branch_advanced_since_execute`.
- [x] Undo moves the ref via compare-and-swap and post-verifies the old tip.
- [x] Undo never touches working-tree files, the index, or other refs.

### C8-E: Confirmation-gated execute for published ranges

**Files:**

- Modify: `crates/super-git-core/src/git/execute_history_edit.rs`
- Modify: `crates/super-git-core/src/model.rs` (confirmation target fields)
- Test: extend `crates/super-git-cli/tests/execute_history_edit.rs`

Acceptance:

- [x] Static confirmation validation reuses the C7-C rule table with
      `history_edit` identity fields and the deterministic phrase.
- [x] A forged or stale confirmation never executes.
- [x] Fresh revalidation runs even with a valid confirmation.
- [x] Successful published-range execute still returns the local undo token
      and records that undo does not un-publish.

## Deferred Features

These are product ideas, not part of the first history edit implementation:

- `edit` and commit `split`
- autosquash from `fixup!`/`squash!` subjects
- `--onto` and rebasing onto a moved base
- editing the root commit
- merge-commit preservation (`--rebase-merges` semantics)
- commit signing support for rebuilt commits
- changing author identity
- editing branches not checked out in the current worktree
- commit-message lint integration (for example, conventional commits)
- batch history edits across multiple branches

## C8-0 Self-review Checklist

- [x] The contract does not allow preview to write.
- [x] Survey mode is read-only and never eligible for execute.
- [x] Execute never invokes interactive rebase or any editor.
- [x] The tree-identity invariant is stated and post-verified.
- [x] Branch ref writes are compare-and-swap only, in execute and undo.
- [x] Published-history rewrites require the separate confirmation artifact.
- [x] Published detection freshness is documented as a limitation.
- [x] Undo restores the branch pointer only and validates provenance.
- [x] Instruction validation failures return structured, repairable reasons.
- [x] Author identity preservation is explicit.
- [x] Signature loss on rewritten commits is surfaced as a warning.
- [x] Future implementation slices are small enough to review independently.
