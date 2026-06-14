# C8-reorder History Edit Reorder Contract

> **Status:** Active contract checkpoint for the `reorder` history-edit op.
> C8-reorder-B preview behavior has landed; execute/undo behavior remains
> blocked until C8-reorder-C. (Filename dated by authoring day, 2026-06-13;
> the original proposal named 06-14.)

**Goal:** Unlock reordering commits in a `history_edit` range, consuming the
Stage 7 replay predictor that C8-drop already proved out.

**Architecture:** `reorder` replays the range's commits in a new order, one
per step, with the same C9 replay shapes as `drop`. Unlike `drop`, reorder is
**tree-preserving by contract**: v0 requires `tree(new tip) == tree(old tip)`
and blocks any reorder that would change the final tree. That single decision
places reorder in the tree-preserving (reword/fold) safety class, not the
tree-changing (drop) class — so it needs none of drop's working-tree
machinery.

---

## Why Now

`drop` (C8-drop-A..D) consumed the per-step replay predictor and the
plan_id-bound prediction evidence, and the C8-drop review hardened both. The
same predictor replays an explicit `(commit, parent)` list onto a base; reorder
is that predictor fed a permuted kept-list. The remaining work is contract, not
core machinery — provided we resist copying drop's *semantics* wholesale.

## What `reorder` Means

`reorder` changes the **order** of the range's commits. The branch ref moves
to a newly built chain whose commits carry the same patches in a new sequence.

Language discipline (documentation and messages must follow it):

- reorder changes the commit **order**, the rebuilt commits' **object ids**,
  and the **intermediate** trees along the chain. It does **not** change the
  final tree, and it does **not** add or remove any commit's patch — that is
  the v0 invariant, enforced as a hard block (below), not a hope.
- reorder is therefore in the same risk class as `reword`/`squash`/`fixup`:
  the worst honest outcome is "history shape changed", never "content changed"
  or "content deleted".
- Undo is the existing tree-preserving shape: restore the branch-tip snapshot.
  No working-tree restore, because the working tree never moved.

## Replay & Prediction Model (Consuming C9)

For a range `base..HEAD` whose commits are replayed in a new order:

```text
tip = base
for each commit in the NEW order:
  merge-tree --write-tree -z --merge-base=<commit's ORIGINAL parent> <tip> <commit>
  clean      -> wrap result tree in an unreferenced synthetic commit, advance tip
  conflicted -> the plan is blocked; record the step evidence
```

This is exactly the C9-0 role-rotation table (`base` = the replayed commit's
original parent, `ours` = the tip built so far, `theirs` = the commit being
replayed), and exactly `drop`'s consumption of
`conflict_prediction::predict_replay_onto`. Notes:

- The 3-way base for each commit is its **original parent**, the patch
  definition `parent..commit` — independent of the new ordering.
- The predictor is reused as-is. The public `predict rebase` CLI verb is the
  wrong surface to consume (it is range-ordered and user-facing); reorder
  feeds the internal explicit-list helper, the same way `drop` does. No core
  change is required for prediction.
- Prediction kind string: `reordered_commit_replay` (drop uses
  `kept_commit_replay`). `dropped_commits` is empty for reorder; the kind plus
  an empty dropped-list distinguishes the two for diagnostics.

## The Two Gates (Spike-Driven)

Reorder is only *contract-safe* because two failure modes are blocked at
preview time, before any write. Both were measured against real Git, not
assumed.

### Spike results (Git 2.x, temp repos, `merge-tree --write-tree`)

**Counterexample — clean steps can still change the final tree.** A revert
pair `base:f=1 -> c1:f=X -> c2:f=1` (original final tree `f=1`), reordered to
`c2, c1`:

```text
step1 replay c2: --merge-base=c1 ours=base theirs=c2  -> rc=0 (clean), tree f=1  (== base: EMPTY step)
step2 replay c1: --merge-base=base ours=<step1> theirs=c1 -> rc=0 (clean), tree f=X
final f.txt = 'X'   original f.txt = '1'   preserved = NO
```

Both steps merge cleanly, yet the reordered final tree differs from the
original. Reorder **can silently change content with no conflict signal** —
the exact surprise class C8-0 excluded. Hence the hard gate below.

**Control — independent reorder preserves the final tree.** `base:a.txt`,
`c1:+b.txt`, `c2:+c.txt`, reordered to `c2, c1`:

```text
step1 rc=0 (not empty)  step2 rc=0   final files=[a.txt b.txt c.txt]  preserved = YES
```

So the feature is meaningful: a genuine permutation of independent commits
preserves the tree, and the gate fires only on non-commuting interactions.

**Conflict — non-commuting same-line edits.** `base:f=1 -> c1:f=A -> c2:f=B`,
reordered to `c2, c1`: `step1 rc=1` (conflict). The existing predictor already
stops at the first conflict and reports per-file stage evidence (C9/drop).

**Raw-tree-as-branch arg** is accepted by current merge-tree (`rc=0`) but stays
undocumented; reorder keeps wrapping clean steps via `git commit-tree`, exactly
as C9-C decided.

### Gate 1 — `reorder_changes_final_tree` (SAFETY)

When every step is clean but `prediction.final_tree != tree(old HEAD)`, the
plan is **blocked** with reason code `reorder_changes_final_tree`. The block
details carry both trees (`old_tree`, `predicted_final_tree`) as evidence.
This is the safety-critical invariant: it is the only thing standing between
"reorder" and "silent content change". It is a pure preview-time check —
`tree(old HEAD)` is the current branch-tip tree — and it is plan_id-bound via
the embedded prediction.

### Gate 2 — `reorder_creates_empty_commit` (HYGIENE, v0 block)

When a replayed commit's predicted tree equals its new parent's tree (the
prior synthetic tip, or `base` for the first step), that step would become an
empty commit in a real rebase. v0 **blocks** it with reason code
`reorder_creates_empty_commit` (details: the offending commit).

Honest framing: unlike Gate 1, an empty step does **not** by itself threaten
the final tree, so this gate is **scope/hygiene, not safety**. It is blocked
in v0 for three reasons:

- It keeps the execute rebuild a clean 1:1 (every step maps to one non-empty
  rebuilt commit; commit count is preserved), reusing drop's
  `rebuild_replayed_commits` without an empty-handling branch.
- It avoids silently making the rebase-`--empty` decision (drop the empty
  commit and change the count, or keep a confusing no-op) inside reorder.
- An empty step signals a non-clean permutation (revert-like interaction),
  which is worth surfacing rather than absorbing.

The two gates correlate but are distinct: the revert-pair triggers **both**
(empty step 1 *and* a changed final tree); a tree-changing reorder need not
have an empty step. When both apply, the plan reports both reason codes (the
existing multi-block pattern), so the agent sees the full picture. A future
slice may relax Gate 2 by adopting an explicit empty-commit policy; Gate 1 is
permanent.

Gate precedence: a predicted conflict (`predicted_conflict`) is reported alone
(no `final_tree` exists to evaluate the other gates). Otherwise both content
gates are evaluated and all that apply are reported.

## Instructions Contract

Input stays `super-git.instructions.v0.1` unchanged. Reorder is expressed by
the **order of the `items` array itself** — no `op: "reorder"` op, no
`position` field. Rationale: a positional op would duplicate and could
contradict the list order; reusing list order keeps one source of truth and
fails closed on older binaries (which still reject non-natural order).

Detection relaxes exactly one existing rule. Today, when the items cover the
range as a set but are not in oldest-first order, validation blocks with
`instructions_order_mismatch`
(`crates/super-git-core/src/git/history_edit.rs`). Reorder unlocks that case:

- Items must still **cover the range exactly as a set** — unknown, duplicate,
  or missing commits keep their existing hard blocks (`instructions_unknown_commit`,
  `instructions_duplicate_commit`, `instructions_incomplete`). Reorder never
  relaxes set-coverage; it only relaxes *sequence*.
- A valid set-cover in non-natural order is a **reorder candidate** instead of
  `instructions_order_mismatch`.
- Because a mistaken shuffle is now a meaningful plan rather than an error, the
  plan must make the reorder loud, via a **new advisory `reorder` block** (not
  `result_summary` — see "Plan id and the advisory block" below):
  - `reorder.commits_reordered`: count of commits whose index differs from the
    natural oldest-first order.
  - `reorder.old_order` and `reorder.new_order`: the explicit oid arrays the
    agent diffs. `old_order` = `range.commits` order; `new_order` =
    `instructions.items` order. Those two bound fields are authoritative; the
    arrays are a derived convenience.
  - effects lead with an explicit line, e.g. "This plan changes commit order on
    `<branch>`." (and never claims the tree changes).

`instructions.order` stays `"oldest_first"`, and the contract pins its meaning:
it is oldest-first **of the planned result history** — i.e. the order of the
`items` array. For every non-reorder op the planned order equals the original
range order, so nothing changes; for reorder the items are already the new
oldest-first sequence, so the value stays accurate. The explicit
`reorder.old_order`/`new_order` arrays remove any residual ambiguity (P3).

### Plan id and the advisory block (compatibility — P1)

The reorder advisory data is **excluded from the plan_id projection**, and the
plan schema stays `super-git.plan.v0.5` with **no bump**. The reasoning is the
binding rule, applied honestly:

- The plan_id must bind exactly what determines the execute result: the
  instructions (which already carry the new **order**) and the `prediction`
  (per-step merged trees + `final_tree`). Both are already hashed. The new
  order and the predicted trees therefore cannot be forged — execute
  re-derives both from fresh state and requires the plan_id to match.
- `reorder.commits_reordered`/`old_order`/`new_order` are **derivable** from
  those already-bound fields (`range.commits` and `instructions.items`), so
  they add no new authority. They belong in the non-hashed advisory tier with
  `effects`, `limitations`, subjects, and author prose — fields execute
  ignores and a tampered plan may set freely without changing the write.
- Concretely: the `reorder` block is a new optional top-level plan field
  (`#[serde(default, skip_serializing_if = "Option::is_none")]`), and it is
  **not** added to the `HistoryEditPlanHashInput` projection. `result_summary`
  is left byte-identical, so existing v0.5 plan ids stay valid.

This is the explicit-compatibility-projection path, chosen over a
`super-git.plan.v0.6` bump precisely because **no field that binds the execute
result changed** — adding `commits_reordered` to the wholesale-hashed
`result_summary` (the naive "v0.5 + serde default" move) would silently change
the projection and re-hash old plans to a `plan_id` mismatch, the exact class
of problem the C8-drop review caught. A B-slice test pins this: tampering with
`reorder.commits_reordered`/`old_order`/`new_order` must **not** change the
plan_id, while tampering with `instructions.items` order or the `prediction`
**must**.

## Op-Mixing Boundary (v0)

A reorder instruction list (non-natural order) may otherwise contain only
`pick` and `reword`:

- `drop` mixed with reorder is blocked (`reorder_with_drop_unsupported`).
  drop *intends* to change the final tree; reorder *forbids* it. The two
  invariants are contradictory in one chain, so v0 does not combine them.
- `squash`/`fixup` mixed with reorder is blocked
  (`reorder_with_fold_unsupported`). Fold reuses original commit trees
  (order-dependent), while reorder rebuilds from replayed trees; mixing two
  rebuild models in one chain multiplies the verification surface for marginal
  first-slice value. This mirrors drop's `drop_with_fold_unsupported`.

Relaxing either later is a pure extension.

## Preview Contract

A valid reorder candidate produces a plan classified as follows:

- **Predicted conflict** at any step: `execution.status: "blocked"`,
  `predicted_conflict`, with the conflicting step's evidence attached (the C9
  step shape). Nothing is ever auto-resolved.
- **Empty step** (Gate 2): `blocked`, `reorder_creates_empty_commit`.
- **Final tree would change** (Gate 1): `blocked`,
  `reorder_changes_final_tree`, with `old_tree`/`predicted_final_tree`.
- **Clean, tree-preserving reorder**: in the *final* contract (after
  C8-reorder-C), a normal write plan gated only by the published-range rule —
  `executable` when unpublished, `preview_only` + confirmation when published.
  **In C8-reorder-B, execute is not implemented**, so the slice must not
  advertise an executable plan (see "B advertisement" below): a clean reorder
  is advertised with `execute_supported: false` and a dedicated
  `reorder_execute_unsupported` advertisement, carrying the full prediction +
  `reorder` block + effects so the agent sees exactly what the reorder would
  do, while execute fail-closes.
- The plan embeds the prediction evidence it was built from (per-step trees,
  `final_tree`), plan_id-bound, exactly like drop.
- Working-tree state does **not** block preview (it stays read-only). It does
  not block execute either (see below) — unlike drop, reorder has no
  clean-tree requirement.

### B advertisement: keep B strictly preview-only (Option A chosen)

There is a real fork for how a *clean* reorder is advertised while execute is
unimplemented (B), because reorder's unpublished tier is `executable` (no
confirmation) — unlike drop, whose always-`preview_only` tier let drop-B
advertise its final tier with `execute_supported: false` and no broken promise.

- **Option A (recommended): keep B strictly preview-only.** A clean reorder is
  advertised as `blocked` with reason `reorder_execute_unsupported` and
  `execute_supported: false`, carrying full evidence. No plan claims
  `executable` or `preview_only` in B. The risk *severity* is still set per
  published state (medium/high) so the policy is visible, but the confirmation
  *artifact mechanism* lands with execute in C. C8-reorder-C removes the
  `reorder_execute_unsupported` advertisement and assigns the real tier.
  Honors "preview must not promise what it can't keep" most strictly.
- **Option B (drop-B-faithful): advertise the final tier now** —
  `executable` (unpublished) / `preview_only` (published) with
  `execute_supported: false`. This fully exercises the confirmation policy in
  B, but advertises `executable` for unpublished reorder, which the lead
  flagged as a possible broken promise.

Lead sign-off chose Option A before C8-reorder-B implementation. The B tests
therefore pin a clean reorder as `blocked` with `reorder_execute_unsupported`,
`execute_supported: false`, no confirmation artifact, and full prediction +
`reorder` evidence.

Plan schema: reorder rides the existing history-edit plan family at
`super-git.plan.v0.5` with no bump. It adds new block codes and one new
**non-hashed** advisory field (the `reorder` block above); it does **not** add
fields to the hashed `result_summary`, so the plan_id projection is unchanged
(see "Plan id and the advisory block"). New plans still fail closed on older
binaries via `deny_unknown_fields`.

Preview's read-only boundary keeps the precise drop wording: refs, index,
working tree, and config are untouched, but the replay prediction may add
unreferenced, gc-collectable objects (synthetic commits per clean step) — the
same nuance `predict rebase` and the drop preview already state.

## Execute Contract (Draft)

Execute trusts nothing from the plan file. As today, it re-derives a fresh
plan — including a fresh replay prediction — from the live repository and
requires the fresh plan_id to match. A moved tip, a changed range, or a
different prediction all surface as plan-invalid.

The decided semantics — **tree-preserving, ref-only** (this is the payoff of
the v0 invariant):

- **Rebuild** uses drop's mechanism: build the new chain with
  `git commit-tree`, one commit per step, using the fresh prediction's per-step
  `merged_tree`s (the intermediate trees differ from the originals because the
  order changed, so the tree-preserving ops' "reuse original tree" path cannot
  be used here). **Unchanged-prefix rule (P2):** a leading commit keeps its
  original object id only where **both** its position is unchanged (the new
  index equals the natural index) **and** its op/message is unchanged (a plain
  `pick`). The prefix ends at the first reordered position **or** the first
  reworded commit, whichever comes first — because a `reword` at its natural
  position still produces a new object id (the message changed), so "position
  unchanged" alone is not sufficient to reuse the id. The preview's
  `result_summary.unchanged_prefix_commits` must apply the identical cap (it
  already caps at the first reword/fold; reorder adds a cap at the first
  reordered position), so preview and execute agree on which ids are preserved.
- **Post-verify is the tree-preserving invariant**: `tree(new tip) ==
  tree(old tip)`. This is the *same* check the existing reword/fold execute
  runs — reorder does not need drop's `final_tree` oracle, because the oracle
  *is* the old tree. A mismatch is an implementation bug and aborts with the
  ref unmoved (the contract already guarantees, via Gate 1 at preview and the
  fresh re-prediction at execute, that a tree-changing reorder never reaches
  this point).
- **No working-tree synchronization.** Because the final tree is identical to
  the old tip's tree, moving only the ref leaves the working tree and index
  valid — the same reason reword/fold touch nothing. Therefore reorder needs
  **none** of drop's machinery: no clean-tree gate, no ignored-path collision
  gate, no `read-tree -u --reset` sync. A dirty working tree is allowed with
  the existing `working_tree_dirty` warning, just like reword/fold.
- **Ref move** is the existing compare-and-swap against the pre-execute tip.
- **Failure window** is the tree-preserving one: a failure after the ref moved
  rolls the ref back (no sync stage exists, so drop's `execute_partial_failure`
  window does not arise). Completed-record write failure rolls back, as today.

## Confirmation & Risk

**Decided (lead sign-off 2026-06-13):**

- **Unpublished reorder**: `executable`, **no confirmation**, medium severity,
  `reversible_if_unchanged`, no human confirmation — identical to unpublished
  reword/fold. (In C8-reorder-B this tier is advertised with
  `execute_supported: false` per the "B advertisement" decision; the
  confirmation *mechanism* is exercised when execute lands in C.)
- **Published reorder**: `preview_only` with the existing
  `super-git.confirmation.v0.1` artifact and the published-rewrite phrase —
  identical to any published history edit.

Rationale: with `tree(new tip) == tree(old tip)` enforced as a hard invariant,
reorder's worst case is "history shape changed", the same class as
reword/fold. drop's "always confirm" rule was justified by *content deletion*,
which reorder forbids; copying it here would be inconsistent with the rest of
the tree-preserving family. **This is the clearest place not to inherit a drop
decision by reflex.**

**Documented alternative (conservative, NOT chosen):** require confirmation for
*all* reorders (a dedicated reorder phrase) and relax after soak. Recorded for
provenance; the lead chose the consistency path above on 2026-06-13, reasoning
that the `tree(new tip) == tree(old tip)` hard gate keeps reorder in the
reword/fold risk class, not drop's content-deletion class.

Risk vocabulary nuance (effects/limitations, not a severity bump): reorder
changes every rebuilt commit's id and the **intermediate** trees, so
`git bisect` results, per-commit build/test outcomes, and author-date ordering
can shift even though the final tree is identical. The plan must state this
plainly so an agent does not assume "same tree" means "same history behavior".

## Undo Contract (Draft)

Same family as the tree-preserving history-edit undo:
`restore_branch_tip_snapshot`. The C8-C invariant holds — the new and old tips
share one tree, so restoring the branch pointer cannot change file content —
so undo moves only the ref and touches neither the working tree nor the index.
drop's `restore_branch_tip_and_worktree` kind is **not** used; inheriting it
would falsely advertise a working-tree restore reorder never performs.

Record consumption and the re-execute lifecycle reuse the existing shared
mechanism unchanged (a successful undo consumes the execution record so the
same plan can be executed again; the C8-drop review made this common to both
undo kinds), so reorder needs no new lifecycle work here.

## Scope Boundary

- `reorder` only. It composes with `pick`/`reword`; `drop` and `squash`/`fixup`
  mixing are blocked (above).
- Merge commits and root commits in range stay blocked (C8-0 / C9-C boundary):
  single-parent 3-way replay cannot model either.
- Predicted conflicts block; there is no automatic conflict resolution, ever.
- One worktree, current branch, current repository — all existing history-edit
  constraints carry over.
- `split`/`edit` remain deferred.

## Pre-B Review (Codex findings, 2026-06-13)

Three findings were raised before C8-reorder-B and **all accepted**; the
contract above is corrected accordingly. No production code changed yet (the
fields they govern are implemented in B), so these are contract corrections:

- **P1 (schema/plan_id).** Correct — adding `commits_reordered` to the
  wholesale-hashed `result_summary` would re-hash existing v0.5 plans to a
  `plan_id` mismatch. Resolution: the reorder advisory data lives in a new
  **non-hashed** `reorder` block (excluded from the plan_id projection), so
  v0.5 stays valid with **no bump** — the honest "explicit compatibility
  projection" path, justified by the binding rule (the order and predicted
  trees are already bound via instructions + prediction; the summary is
  derived). See "Plan id and the advisory block".
- **P2 (unchanged prefix).** Correct — "position unchanged" alone is wrong when
  reorder mixes with `reword`. Fixed to "position **and** op/message unchanged",
  with the prefix capped at the first reordered position or first reworded
  commit. See the Execute Rebuild bullet.
- **P3 (`instructions.order` ambiguity).** Correct — pinned to mean oldest-first
  of the **planned** history, with explicit `reorder.old_order`/`new_order`
  arrays removing residual ambiguity.

## Open Questions

- **Empty-step policy beyond v0.** If a later slice relaxes Gate 2, it must
  choose drop-the-empty (count changes — drifts toward drop semantics) vs
  keep-the-empty (a no-op commit). Deferred; Gate 1 is unaffected either way.
- **`commits_reordered` definition.** Count of commits whose index differs from
  the natural order is the obvious metric, but a single swap of two adjacent
  commits "moves" two commits by that metric; the human summary should show
  old-order/new-order explicitly so the count is never the only signal.
- **B advertisement.** Resolved before implementation: Option A is chosen.
  Clean reorder preview stays `blocked` with `reorder_execute_unsupported`;
  C8-reorder-C removes that block and assigns the final executable/preview-only
  tier.

## Slice Plan

- [x] **C8-reorder-A** — this contract checkpoint plus the Git-behavior spikes
      that fix the two gates (docs only; no production behavior change).
- [x] **C8-reorder-B** — preview accepts a reordered instruction list: relax
      `instructions_order_mismatch` into reorder detection (set-cover preserved),
      op-mixing blocks (`reorder_with_drop_unsupported`,
      `reorder_with_fold_unsupported`), the two content gates
      (`reorder_changes_final_tree`, `reorder_creates_empty_commit`),
      `predicted_conflict` blocks with step evidence, `commits_reordered` +
      old/new-order summary + effects, plan_id-bound prediction. Execute stays
      rejected for reorder plans until C.
- [ ] **C8-reorder-C** — execute (drop-style replay rebuild + tree-preserving
      post-verify, ref-only move, no worktree sync), undo
      (`restore_branch_tip_snapshot` reuse), public docs
      (README / command-reference / safety-model), and lifecycle hardening.
- Alternative: if C grows (e.g. the unchanged-prefix rebuild or the
  old/new-order evidence needs more surface than expected), split execute (C)
  from undo + public docs + hardening (D), mirroring the drop series.
