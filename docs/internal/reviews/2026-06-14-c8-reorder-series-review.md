# C8-reorder Series Integrated Review

Review date: 2026-06-14

Scope:

- Base: `60c47a3^`
- Head: `3b35825` (`develop`)
- Commits reviewed: C8-reorder-A contract, contract correction, C8-reorder-B preview, C8-reorder-C execute/undo/docs, and the develop merge.

Verdict: no P0 found. The core ref-only reorder execute path dogfooded cleanly, but the review found one P1 confirmation-design gap and several P2/P3 contract and documentation issues that should be fixed before treating the reorder series as fully closed.

## Findings

### P1 - Published history-edit confirmation phrases are reusable across distinct same-tip plans

Evidence:

- `crates/super-git-core/src/git/preview_history_edit.rs:425` builds the non-drop published phrase from only `branch_ref + tip_commit`.
- `crates/super-git-core/src/git/execute_history_edit.rs:621` validates the same phrase.
- `crates/super-git-core/src/git/execute_history_edit.rs:567` validates `confirmation.plan_id`, but that field is machine-readable JSON, not the human-typed phrase.
- `docs/command-reference.md:492` documents the reusable published phrase shape.
- `crates/super-git-cli/tests/execute_history_edit.rs:540` bakes the same phrase into the new published reorder path.

Impact:

Two different published history-edit plans over the same branch tip can have different `plan_id`s but the same required typed phrase. A tool can therefore ask the human to type a phrase while showing one plan, then place that same phrase into a confirmation artifact for another same-tip plan. The JSON `plan_id` still protects against stale artifacts, but the human acknowledgement does not prove awareness of the exact plan being executed.

Suggested fix:

Make every history-edit typed phrase include a plan-bound value, such as a short plan-id prefix or a dedicated instruction/prediction digest:

```text
rewrite published history on <branch_ref> at <tip_commit> for plan <short-plan-id>
drop <N> commit(s) from <branch_ref> at <tip_commit> for plan <short-plan-id>
```

Update preview, execute validation, public docs, internal contracts, and tests. Add a negative test with two same-tip published plans where the confirmation JSON carries plan B's `plan_id` but the phrase from plan A; execution should fail with `confirmation_phrase_mismatch` and leave the ref unmoved.

### P2 - Blocked tree-changing reorder still reports `result_summary.final_tree_unchanged: true`

Evidence:

- `crates/super-git-core/src/git/history_edit.rs:511` sets `final_tree_unchanged` from `commits_dropped == 0`.
- `crates/super-git-core/src/git/preview_history_edit.rs:255` detects clean replay reorders whose predicted final tree differs from the old tip and blocks them with `reorder_changes_final_tree`.
- `crates/super-git-core/src/git/preview_history_edit.rs:545` exposes the earlier summary unchanged.

Reproduced in a disposable temp repo with a revert-pair reorder: preview returned `execution.status: "blocked"` with both `reorder_changes_final_tree` and `reorder_creates_empty_commit`, while the same JSON reported `result_summary.final_tree_unchanged: true`.

Impact:

The write is correctly blocked, so this is not an execution safety hole. It is still a machine-contract inconsistency: one plan field says the final tree is unchanged while the blocked reason says the final tree would change. Agents that read the summary before the blocked reasons can misunderstand why the plan is unsafe.

Suggested fix:

For reorder candidates, derive the exposed `result_summary.final_tree_unchanged` from the replay prediction when available, not from `commits_dropped == 0`. At minimum, make a blocked `reorder_changes_final_tree` plan expose `final_tree_unchanged: false`. Add a regression test to the revert-pair fixture asserting the summary and blocked reason agree.

### P2 - Intent record cleanup failures are swallowed after CAS failure or rollback success

Evidence:

- `crates/super-git-core/src/git/execute_history_edit.rs:171` ignores `fs::remove_file(&record_path)` after a compare-and-swap ref failure.
- `crates/super-git-core/src/git/execute_history_edit.rs:991` ignores `fs::remove_file(record_path)` after rollback successfully restores the previous tip.

Impact:

In both paths the branch effect is absent or restored, but a stale `intent` record can remain if cleanup fails. The user sees only the original error, then the same plan can be permanently blocked by the replay guard as `execution_already_attempted`; undo also cannot help because the record is not completed. This is the same class of "effect state and record state disagree" issue that the drop review treated as important.

Suggested fix:

Report cleanup failure as a structured partial-failure/recovery case instead of swallowing it. Include at least `execution_record_path`, observed branch tip, and a `safe_next` hint explaining that the branch effect is absent/restored but the stale intent record blocks re-execute until cleaned up. Add a Unix regression test that makes the executions directory non-writable, triggers rollback, and verifies the structured error plus preserved stale record.

### P2 - Public docs still over-generalize history-edit executability and tree preservation

Evidence:

- `README.md:68` says unpublished ranges produce an executable plan, but unpublished `drop` plans are always `preview_only` and confirmation-gated.
- `docs/roadmap.md:175` still explains history-edit undo as branch-pointer snapshot only.
- `docs/roadmap.md:206` says the first op set never changes any tree and never touches files, which is now false for `drop`.
- `crates/super-git-core/src/git/execute_history_edit.rs:40` emits `confirmation_required` text that mentions only "published range", even for unpublished `drop`.
- `crates/super-git-core/src/git/execute.rs:172` says confirmation artifacts are supported only for worktree_remove and published-range history_edit plans, again omitting `drop`.

Impact:

The code now has three distinct history-edit classes: tree-preserving ref-only edits, tree-preserving reorder with replay prediction, and tree-changing drop with clean-worktree sync. The docs and error strings still sometimes describe only the pre-drop world. This can lead agents to expect direct execution for unpublished drop or assume every undo is snapshot-only.

Suggested fix:

Tighten the public wording around the split:

- unpublished tree-preserving ranges execute directly;
- published ranges and all drop plans require confirmation;
- tree-preserving edits/reorder are ref-only and dirty-worktree tolerant;
- drop is tree-changing, always confirmation-gated, clean-worktree required, and syncs index/worktree.

Update the two error messages to say "confirmation-gated history_edit plan" or "published ranges and drop plans".

### P3 - C8-0 internal contract hard-block table is stale for drop and reorder

Evidence:

- `docs/internal/plans/2026-06-10-c8-0-history-edit-preview-contract.md:80` correctly states that drop and reorder are now implemented through their own checkpoints.
- `docs/internal/plans/2026-06-10-c8-0-history-edit-preview-contract.md:259` still says `instruction_op_unsupported` covers `drop` and reorder.
- `docs/internal/plans/2026-06-10-c8-0-history-edit-preview-contract.md:263` still says `instructions_order_mismatch` means reordering is not supported yet.

Impact:

This is internal documentation, but it is exactly the kind of contract table agents search when debugging blocked history-edit plans. The top status update and the reason-code table now disagree.

Suggested fix:

Split the table by historical C8-0 baseline vs current status, or update the active entries:

- `drop` is supported via the drop checkpoint and blocked only for unsupported mixes or prediction conflicts.
- reorder is supported via the reorder checkpoint when prediction preserves the final tree.
- `instructions_order_mismatch` remains relevant only for malformed/non-covering/non-reorder cases, if at all.

### P3 - Reorder contract names a non-existent public `result_summary.unchanged_prefix_commits` field

Evidence:

- `docs/internal/plans/2026-06-13-c8-reorder-history-edit-contract.md:321` says preview's `result_summary.unchanged_prefix_commits` must apply the same cap as execute.
- The public plan schema at `crates/super-git-core/src/model.rs:850` does not expose `unchanged_prefix_commits`.
- `crates/super-git-core/src/git/preview_history_edit.rs:545` maps only the visible result-summary fields.
- `crates/super-git-core/src/git/preview_history_edit.rs:870` includes `result_summary` in the plan-id projection, so adding this field later is a schema/projection decision.

Impact:

Future agents may expect a JSON field that is not emitted, or a later change may add the field as if it were a docs-only cleanup and accidentally re-hash v0.5 plans.

Suggested fix:

Either change the internal contract to say internal `program.summary.unchanged_prefix_commits`, or intentionally expose the field with an explicit compatibility decision and plan-id test.

### P3 - Advisory reorder fields are intentionally forgeable but need a public note and execute-side regression

Evidence:

- `docs/internal/plans/2026-06-13-c8-reorder-history-edit-contract.md:194` says the top-level `reorder` block is non-hashed advisory data.
- `crates/super-git-core/src/git/execute_history_edit.rs:56` correctly runs execution from a fresh preview and authoritative instructions/prediction, not the submitted advisory fields.

Impact:

This is executable-neutral, but a forged `reorder.old_order`, `new_order`, or `commits_reordered` can mislead a reader of the plan JSON. The internal docs explain this; public command-reference does not, and current integration coverage does not prove execute ignores forged advisory reorder fields.

Suggested fix:

Add a CLI execute regression test that tampers `data.reorder.*` and `effects`, then confirms execute follows `instructions.items` and fresh prediction. Add one public sentence that `reorder` is display evidence only; execution authority comes from plan-id-bound `instructions.items` plus `prediction`.

## Verification Notes

Subagent lanes covered:

- core execute/undo safety;
- preview/plan-id/schema;
- docs/contract/public UX;
- dogfood/real CLI behavior;
- adversarial tampering.

Dogfood and temp-repo checks found no actionable issue in clean reorder preview -> execute -> undo, dirty worktree preservation, predicted-conflict blocking, or published confirmation enforcement. Local full gates were reported green by the core reviewer: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `git diff --check`. The main repo remained clean before this review artifact was written.
