# C8-drop History Edit Drop Contract

> **Status:** Active contract checkpoint for the first tree-changing
> history-edit op. No `drop` preview, execute, or undo behavior exists yet;
> this document fixes the contract before any of it is implemented.

**Goal:** Unlock the `drop` instruction op for `history_edit`, consuming the
Stage 7 per-step conflict prediction that C8-0 declared a prerequisite.

**Architecture:** `drop` removes a commit's patch from the final history by
replaying the kept commits, one per step, with the C9 rebase-chain predictor
shapes as both the preview evidence and the execute post-verify oracle. It is
the first history-edit op where the final tree differs from the original tip
tree, and — by explicit decision — the first whose execute synchronizes the
working tree, with real-rebase semantics.

---

## Why Now

C8-0 excluded `drop` with a precise reason: any drop changes the final tree
and silently reverts content without a conflict signal, and no conflict
prediction existed. That prerequisite is now met:

- C9-A/C9-C provide per-step replay prediction with explicit merge bases
  (`--merge-base=<parent>`), stop-at-first-conflict, and a predicted
  `final_tree` when every step is clean.
- C9-B/C9-D validated those JSON shapes through real CLI use.

The roadmap names a safe `drop` for wip commits as the highest-demand
consumer of prediction. This contract is that consumer's design.

## What `drop` Means

`drop <commit>` removes that commit's **patch** from the final history: the
branch tip moves to a newly built commit chain in which the kept commits are
replayed in order and the dropped commit is absent.

Language discipline (documentation and messages must follow it):

- `drop` does **not** delete the commit object, and it does not "delete
  history". The original commits remain in the object database, reachable
  from the reflog and the undo token's recorded tip, until Git's normal gc
  horizon passes.
- What changes is exactly one thing the user owns: which chain the branch
  ref points at.
- Undo is therefore the existing history-edit shape: restore the branch tip
  snapshot (plus the working-tree restore described below).

## Replay Model (Consuming C9)

For a range `base..HEAD` with instructions containing `drop`:

```text
kept = range commits, oldest first, minus the dropped ones
tip  = base
for each kept commit:
  merge-tree --write-tree -z --merge-base=<commit's original parent> <tip> <commit>
  clean      -> wrap tree in an unreferenced synthetic commit, advance tip
  conflicted -> the plan is blocked; record the step evidence
```

This is exactly the C9-C role-rotation model. Two notes:

- The 3-way base for each kept commit is its **original parent** — which may
  itself be a dropped commit. That is correct cherry-pick semantics: the
  patch being replayed is `parent..commit`.
- The C9 core exposes range-based `predict_rebase_chain(base, onto)` today.
  The preview needs an internal variant that replays an **explicit
  (commit, parent) list** (the kept commits) onto base. This is a
  `super-git-core`-internal helper extension, not a new public surface.

Dropping every commit in the range is allowed and meaningful ("abandon the
whole wip range"): the kept list is empty, the predicted final tree is
base's tree, and the new tip is `base` itself.

### Op-Mixing Boundary (v0)

An instruction list containing `drop` may otherwise contain only `pick` and
`reword` in this design. Combining `drop` with `squash`/`fixup` in one list
is blocked (`drop_with_fold_unsupported`): fold groups reuse original trees,
replayed commits get new trees, and mixing the two rebuild models in one
chain multiplies the verification surface for marginal first-slice value.
Relaxing this later is a pure extension.

## Preview Contract

Input stays `super-git.instructions.v0.1` — `drop` extends the op
vocabulary, not the document shape. Older super-git versions reject the
unknown op with a blocked plan, which fails closed.

A list containing at least one `drop` produces a **tree-changing plan**,
classified and gated as follows:

- The preview runs the replay prediction internally.
- **Predicted conflict** at any step: `execution.status: "blocked"` with
  reason code `predicted_conflict`, and the conflicting step's evidence
  (commit, per-file stages — the C9 step shape) attached so the agent can
  see exactly which kept commit collides with which file. Prediction is
  guidance; nothing is resolved automatically, ever.
- **All steps clean**: the plan is confirmation-gated, reusing the exact
  mechanism C8 already has for published ranges:
  `execution.status: "preview_only"` with
  `requires_confirmation_artifact: true` and confirmation reason code
  `tree_changing_drop` (alongside `published_history` when that also
  applies).
- The plan embeds the prediction evidence it was built from:
  `prediction.final_tree` (the predicted post-edit tree), per-step
  summaries, and the predicted new-tip chain length. A tree-changing plan
  without a `final_tree` is never executable — that field is the execute
  oracle, not advisory metadata.
- Working-tree state does not block **preview** (it stays read-only), but a
  dirty tree is surfaced as a hard-block **for execution** in the plan (see
  below), not just a warning as in the tree-preserving ops.

Plan schema: the additions ride on the existing history-edit plan family
with new fields under `#[serde(default)]`. New plans fail closed on old
binaries (`deny_unknown_fields`); old plans keep loading. The prediction
evidence participates in the plan_id projection (it is plan-binding, not
advisory): execute re-derives it from fresh state, so a tampered
`final_tree` produces a plan_id mismatch instead of a forged oracle.

## Execute Contract

Execute trusts nothing from the plan file. As today, it re-derives a fresh
plan — including a fresh replay prediction — from the live repository and
requires the fresh plan_id to match the submitted plan's. A moved tip, a
changed range, or a different prediction all surface as plan-invalid, never
as a silently different rewrite.

The decided working-tree semantics (real-rebase semantics):

- The tree-preserving ops could move only the ref because the old and new
  tips share one tree. `drop` cannot: moving only the ref would leave the
  working tree and index at the old tree, making the dropped content
  reappear as staged changes — "dropped, but still in every file".
- Therefore `drop` execute **requires a clean working tree** (hard block
  `working_tree_dirty`, unlike the tree-preserving ops which allow dirty),
  and after the ref moves it **synchronizes the index and working tree to
  the new tip**. This is the first history-edit action that touches the
  working tree; the write-boundary documentation must say so explicitly
  when this lands.

Order of operations:

1. Re-derive and match plan_id (includes fresh prediction evidence).
2. Verify clean working tree and no in-progress operation.
3. Validate the confirmation artifact (always required; next section).
4. Build the kept chain with `git commit-tree`, preserving each original
   author, using the merged trees from the fresh prediction steps.
5. **Post-verify before the ref moves:** `tree(new chain tip) ==
   fresh prediction final_tree`. This replaces the tree-preserving
   invariant `tree(new tip) == tree(old tip)`, which cannot hold for drop.
   A mismatch is an implementation bug and aborts with nothing moved.
6. Write the intent record, move the branch ref with the existing
   compare-and-swap against the pre-execute tip.
7. Synchronize index and working tree to the new tip.
8. Complete the execution record.

Rollback policy — stated precisely, because the working tree changes what
"rollback" can honestly promise:

- **Failure after the ref moved but before sync (step 7) started:** the
  branch ref is rolled back to the pre-execute tip with compare-and-swap.
  The working tree and index were never touched, so this genuinely is the
  pre-execute state.
- **Failure once sync has started:** the index and working tree may be
  partially mutated, so a ref rollback alone must **not** be described as
  restoring the pre-execute state. This case is an `ExecutePartialFailure`
  contract, not a rollback: the envelope reports the observed branch ref,
  whether index/working-tree sync was attempted and how far it got, whether
  automatic undo is available (it is not, in this state), and a `safe_next`
  recovery hint (the new tip is correct in the ref; the remaining repair is
  finishing or cleanly redoing the sync). This mirrors the worktree_remove
  partial-failure precedent.

In short: rollback restores the branch ref when possible; sync failures
after working-tree mutation become partial-failure recovery cases.

### Sync Primitive (decided 2026-06-12, C8-drop-C)

The sync step is **`git read-tree -u --reset <new tip>`**, chosen over
`reset --hard` and `checkout -f`:

- **Ref isolation.** `read-tree` never touches refs, so the CAS ref move
  stays the only ref write in the whole execute path. `reset --hard` would
  re-write the branch ref through HEAD (a second, non-CAS ref touch plus an
  extra reflog entry); `checkout -f` adds the branch-switching surface and
  runs the `post-checkout` hook. `read-tree` runs no hooks.
- **Spike-verified behavior** (git 2.x, temp repos): file deletions and
  revivals both materialize exactly (a dropped "add" vanishes from the
  working tree; a dropped "delete" revives the file); after sync,
  `status --porcelain --untracked-files=all` is empty. With
  `sparse-checkout` active, paths outside the sparse cone are not
  materialized and keep their skip-worktree bits — same behavior as
  `reset --hard`.
- **Failure semantics.** An `index.lock` conflict fails before anything
  changes (index and working tree both untouched — verified). Later
  failures (disk errors mid-update) can leave a partially synchronized
  index/working tree; that is precisely the partial-failure window above.
  Execute verifies the sync by requiring an empty
  `status --porcelain --untracked-files=all` afterwards.

The clean-working-tree gate is checked with the same status read
(`--untracked-files=all`): untracked files count as dirty, because a
dropped commit that deleted a path revives that path on sync and would
silently overwrite an untracked file sitting there. The execute-side
rejection surfaces as an `execute_precondition_mismatch` on field
`working_tree_clean`.

**Ignored files are a separate gate, not a free pass.** The status read
does not list ignored files, but `read-tree -u --reset` silently overwrites
an ignored untracked file whose path the new tip tracks (spike-verified:
force-add an ignored file in commit A, delete it in commit B, drop B with a
local ignored file at that path — the local content is replaced). Blocking
every ignored file would break normal workflows (`node_modules`, build
output), so execute instead runs a targeted collision check after `new_tip`
is built and **before the CAS ref move**: ignored untracked paths
(`ls-files -i -o --exclude-standard -z`) are compared against the new tip's
tracked paths (`ls-tree -r -z --name-only`), and an exact path match, an
ignored file squatting on a new tracked directory, or an ignored directory
squatting on a new tracked file all hard-block as
`execute_precondition_mismatch` on field `ignored_path_collision`. A
refusal changes nothing — no ref, index, or working-tree write has happened
yet.

Failure envelope: once the ref has moved, any failure in sync or in
completing the execution record surfaces as code
`execute_partial_failure` carrying the observed branch tip, whether sync
completed, the execution-record path, and a `safe_next` hint. The record
stays in `intent` state on a sync failure, so undo (which requires a
`completed` record) and re-execute (`execution_already_attempted`) both
fail closed; the branch ref is already correct at the new tip and the
remaining repair is finishing the sync.

## Confirmation Policy

**`drop` always requires the confirmation artifact**, regardless of
published state. Rationale:

- Content-deletion semantics: a drop silently removes changes from the
  final history with no conflict signal — exactly the surprise class C8-0
  excluded. Published-ness is orthogonal to that risk.
- Tree-changing rewrites are categorically riskier than the
  tree-preserving ops, whose invariant makes the worst case "messages
  changed".
- Starting strict and relaxing later (e.g. unpublished single-wip-commit
  drops) is cheap; the reverse direction is a trust incident.

The artifact reuses `super-git.confirmation.v0.1` unchanged — it is
phrase-based and plan-bound, which is all drop needs. The phrase is
deterministic and produced by a function shared between preview and execute
(the established anti-drift pattern):

```text
drop <N> commit(s) from <branch_ref> at <tip_commit>
```

When the range is also published, `confirmation_reason_codes` lists both
`tree_changing_drop` and the published-history code, but the phrase stays
the drop phrase: one plan, one phrase, the most severe semantic named.

## Undo Contract

Same family as existing history-edit undo — restore the branch-tip snapshot
with compare-and-swap from the recorded new tip back to the recorded
previous tip, validated against the local execution record's provenance.
Drop-specific deltas:

- Because execute synchronized the working tree, undo must too: it requires
  a clean working tree, restores the ref, then synchronizes index and
  working tree back to the previous tip. The undo token uses a new kind
  (`restore_branch_tip_and_worktree`), so older binaries fail closed with
  `unsupported_undo_kind` instead of half-restoring.
- The ignored-path collision gate applies symmetrically (decided in
  C8-drop-D): undo's sync target is the **pre-execute** tip, so an ignored
  untracked file sitting where that tip tracks a path (for example, a fresh
  file created where the dropped commit's force-added path is about to
  revive) hard-blocks before any write, with the same three collision
  shapes as execute. Failures after the ref restore report as
  `undo_partial_failure` — the same honesty split as execute's
  partial-failure window.
- A successful undo consumes the execution record. The record's purpose is
  preventing the same effect from applying twice; once the effect is
  reverted, the identical plan (plan ids are state-based) must be able to
  execute again, or the edit would be locked out of that branch forever.
- Documentation must state plainly: **undo restores the branch ref (and
  working tree); the dropped content's original commits may remain in the
  object database, and undo neither depends on deleting them nor attempts
  to.** That is what makes undo safe — the previous tip is still there.
- The existing limitation stands verbatim: local undo cannot un-publish
  remote history.

## Scope Boundary

- `drop` only. Reordering is excluded (it is the same replay machinery, but
  it gets its own checkpoint once drop has soaked).
- Merge commits and root commits in range stay blocked (C8-0/C9-C
  boundary).
- Predicted conflicts block; there is no automatic conflict resolution,
  ever.
- One worktree, current branch, current repository — all existing
  history-edit constraints carry over.
- `split`/`edit` remain deferred.

## Slice Plan

- [x] **C8-drop-A** — this contract checkpoint (docs only).
- [x] **C8-drop-B** — preview accepts `drop`: instruction validation
      (op mixing rule), internal explicit-list replay prediction
      (`conflict_prediction::predict_replay_onto`), blocked
      `predicted_conflict` plans with step evidence, confirmation-gated
      clean plans with embedded plan_id-bound `final_tree`. Drop plans
      advertise `execute_supported: false` and execute rejects them with
      `tree_changing_execute_unsupported` until C8-drop-C. The
      clean-working-tree execute requirement is a non-volatile
      precondition (`working_tree_clean_required_at_execute:
      enforced_at_execute`) so plan_id does not wobble with dirty state.
- [x] **C8-drop-C** — execute: fresh-prediction re-derivation (the fresh
      preview embeds a fresh replay prediction, and the plan_id match plus an
      explicit `fresh_prediction.final_tree` comparison reject drift),
      always-on confirmation with the drop phrase, chain rebuild from the
      prediction's per-step `merged_tree`s (unchanged prefix before the first
      drop keeps original ids), `final_tree` verified both before the ref
      moves and in post-verify, CAS ref move, `read-tree -u --reset`
      working-tree synchronization with a clean-tree hard gate
      (untracked counts as dirty) plus the ignored-path collision gate
      (safety follow-up — see the sync-primitive section), rollback before
      sync starts, and
      `execute_partial_failure` envelopes after the ref moved
      (record stays `intent`, undo/re-execute fail closed). Undoing a drop
      execution fails closed with `unsupported_undo_kind` until C8-drop-D.
- [x] **C8-drop-D** — undo (`restore_branch_tip_and_worktree`): the
      symmetric inverse of drop execute — clean-tree gate (untracked counts
      as dirty) and ignored-path collision gate against the **pre-execute**
      tip, both before any write; CAS ref restore; `read-tree -u --reset`
      sync back to the pre-execute tip; sync failures after the ref restore
      report as `undo_partial_failure` (never called a rollback). A
      successful undo **consumes the execution record** so the identical
      plan/confirmation can re-execute — without that, state-based plan ids
      would lock the same edit out of the branch forever (this applies to
      the tree-preserving family too). Empty-ignored-directory policy: not
      blocked — invisible to `ls-files -o`, nothing to lose, and
      `read-tree -u` removes it with normal checkout semantics; a directory
      with content is always caught via its files' prefixes. Public docs
      (README, command-reference drop flow, safety-model) updated; lifecycle
      hardening covers execute→undo→re-execute, dirty/untracked/collision
      refusals in both directions, and token-kind downgrade rejection.
