# C9-0 Conflict Prediction Contract

> **Status:** Active contract checkpoint for Stage 7 conflict prediction.
> Scope is the read-only prediction core only. No standalone merge/rebase
> preview, no `drop`/reorder history-edit ops, no branch refresh, and no
> automatic conflict resolution exist yet or are added by this contract.

**Goal:** Bring `git merge-tree`-based conflict prediction inside the
super-git contract: a deterministic, machine-readable answer to "if these two
commits were merged, what would conflict?" — before any feature that needs
that answer is built on top of it.

**Architecture:** A read-only predictor in `super-git-core` runs
`git merge-tree --write-tree -z` against two resolved commits, parses the
NUL-delimited output into a typed JSON shape, and reports per-file predicted
conflicts. Consumers (Stage 6 `drop`/reorder, standalone merge/rebase
previews, safe branch refresh) reuse the same shapes.

---

## Why Stage 7 Needs This

Every remaining high-demand write flow is gated on one question super-git
cannot answer today:

- history-edit `drop` and reordering re-apply patches, so they can conflict.
  C8-0 excluded them *because* prediction did not exist.
- standalone merge/rebase previews are only honest if they can say "this will
  conflict on these files" before anything runs.
- safe branch refresh (fast-forward or rebase-style) needs to refuse or warn
  ahead of time, not fail halfway.

Agents currently discover conflicts by *causing* them: run the merge/rebase,
hit conflict markers, panic, and often corrupt state trying to escape. The
product answer is the same one super-git always gives: move the discovery to
a read-only step with a stable contract.

`git merge-tree --write-tree` (Git >= 2.38) is the right primitive: it
performs a real recursive merge of two commits **in the object database
only** and reports the resulting tree plus conflict details, without touching
the working tree, the index, or any ref.

## What Is Predicted (v0)

One commit pair per call, merge semantics:

```text
predict(ours, theirs) =
  the result of `git merge-tree --write-tree <ours> <theirs>`
```

- **clean**: the merge would succeed; the merged tree OID is reported.
- **conflicted**: the merge would stop on conflicts; the per-file conflict
  list and the conflicted-tree OID (with conflict markers in blobs) are
  reported.

Batch prediction, rebase-chain prediction (replaying N commits), branch
refresh, and history-edit integration are explicit non-goals of v0. They are
consumers, and they compose this primitive.

## Terminology

The contract names the three-way merge participants explicitly, because the
same shapes must later serve rebase-style replays where the roles rotate:

| Term         | Merge prediction (v0)                  | Future rebase-step reuse                |
| ------------ | -------------------------------------- | --------------------------------------- |
| `ours`       | side 1, e.g. the current branch tip    | the tip being built (already-replayed)   |
| `theirs`     | side 2, the branch being merged in     | the single commit being replayed         |
| `merge_base` | best common ancestor (informational)   | the replayed commit's parent             |
| merged tree  | predicted result tree                  | predicted tree for the replayed commit   |

`ours`/`theirs` follow Git's merge-tree argument order (`<branch1>` =
ours, `<branch2>` = theirs). The reported `merge_base` is informational: it
comes from `git merge-base` and names one best ancestor, while merge-tree's
internal recursive merge may combine multiple bases. The prediction result is
authoritative; the reported base is context.

## Read-Only Boundary (Honest Version)

The predictor never touches the working tree, the index, any ref, or any
configuration. But "read-only" must not be overstated:

- `git merge-tree --write-tree` **writes objects** (trees, and blobs with
  conflict markers) into the local object database. They are unreferenced,
  invisible to all normal operations, and garbage-collectable; nothing in the
  repository points at them.
- This is the same class of side effect as `git merge-base` warming caches:
  state that Git owns and reclaims, with no user-visible meaning.

The contract therefore says: **prediction is read-only with respect to
refs, index, working tree, and config; it may add unreferenced,
gc-collectable objects to the object store.** Documentation must use this
phrasing, not an unqualified "read-only".

The predictor runs through the existing hardened `Git` wrapper (ambient env
scrub, `core.fsmonitor=false`, `GIT_OPTIONAL_LOCKS=0`), and is not a raw Git
command wrapper: callers pass revisions, never flags, and the predictor
resolves them to commit OIDs (via `rev-parse --verify --end-of-options
<rev>^{commit}`) before handing anything to merge-tree.

## Result Or Error: The Decision

A prediction call has three outcome classes, and the contract assigns each
one a fixed channel:

1. **Prediction succeeded** — the pair merges cleanly *or* conflicts. Both
   are successful predictions: `{ok: true}` with
   `prediction.status: "clean" | "conflicted"`. A predicted conflict is a
   *result*, not an error — that is the whole point of the feature.
2. **Inputs cannot be predicted over** — unknown revision, revision is not a
   commit, no common ancestor, Git too old for `merge-tree --write-tree`:
   `{ok: false}` with a structured error
   (`error.code`: `rev_not_found`, `no_merge_base`,
   `merge_tree_unsupported`, ...). These are input/environment preconditions,
   exactly the class previews report via structured errors today.
3. **Git itself failed** — merge-tree exits with anything other than 0 or 1:
   `{ok: false}` with the existing `git_command_failed` envelope.

The `blocked`-plan shape is **not** used: blocked plans express "this write
would be unsafe in the current repository state", and prediction performs no
write and depends on no working-tree state. There is nothing to block; there
are only answers and unanswerable inputs.

## JSON Schema Draft

`super-git.conflict-prediction.v0.1`, emitted inside the standard
`{ok: true, data}` envelope:

```json
{
  "schema_version": "super-git.conflict-prediction.v0.1",
  "prediction_kind": "merge",
  "repository": "/abs/worktree/root",
  "inputs": {
    "ours": { "rev": "main", "commit": "<oid>" },
    "theirs": { "rev": "feature", "commit": "<oid>" },
    "merge_base": "<oid or null>"
  },
  "prediction": {
    "status": "conflicted",
    "merged_tree": "<oid>",
    "conflicted_files": [
      {
        "path": "src/lib.rs",
        "stages": [
          { "stage": 1, "mode": "100644", "object": "<oid>" },
          { "stage": 2, "mode": "100644", "object": "<oid>" },
          { "stage": 3, "mode": "100644", "object": "<oid>" }
        ]
      }
    ],
    "notes": [
      {
        "kind": "CONFLICT (contents)",
        "paths": ["src/lib.rs"],
        "message": "<localized free text, advisory only>"
      }
    ]
  },
  "limitations": [
    "prediction is commit-level: the index and working tree are ignored",
    "a rebase replays commits one by one; its conflicts can differ from this single merge prediction",
    "note messages are localized free text; only 'kind' and 'paths' are stable"
  ]
}
```

### Per-File Conflict Shape

`conflicted_files` is derived from merge-tree's conflicted-file-info section,
which is locale-independent plumbing output. Each entry groups the index
stages for one path:

- stage 1 = base, stage 2 = ours, stage 3 = theirs.
- Missing stages identify the conflict shape mechanically: a modify/delete
  conflict has no stage 3 (or no stage 2); a both-added conflict has no
  stage 1. Consumers branch on stage presence, never on message text.
- `mode`/`object` are kept losslessly so a future `drop`/reorder preview can
  show or hash exactly what conflicts without re-running merge-tree.

`notes` carries merge-tree's informational-message section. The `kind` token
(`Auto-merging`, `CONFLICT (contents)`, `CONFLICT (modify/delete)`, ...) and
the path list are stable across locales (verified against a localized Git);
the trailing `message` is translated free text and is advisory display
material only. Nothing in super-git may parse it, hash it, or branch on it.

### Parsing Contract

The predictor invokes `git merge-tree --write-tree -z <ours-oid>
<theirs-oid>` and parses NUL-delimited tokens, never lines or localized
text:

```text
<toplevel-tree-oid> NUL
<mode> SP <object> SP <stage> TAB <path> NUL    (zero or more)
NUL                                              (empty token: section break)
<N> NUL <path>{N} NUL <kind> NUL <message> NUL   (zero or more stanzas)
```

- exit 0 = clean (output is just the tree OID), exit 1 = conflicted,
  anything else = error channel (class 3 above).
- The parser is a pure function over the captured output, unit-tested
  against fixture strings including a localized-message fixture, so Git
  output drift breaks tests instead of production parsing.
- Non-UTF-8 paths currently pass through the same lossy `String` boundary as
  `status.conflicts` (a known, deliberately deferred JSON boundary).

## CLI Naming (Proposal, Not In This Slice)

Prediction is a read, not a plan: it has no `plan_id`, nothing to execute,
nothing to undo. Putting it under `preview` would imply the
preview->execute lifecycle and break the "preview emits plans" rule. The
proposed surface is a new read verb:

```bash
super-git predict merge --ours <rev> --theirs <rev>   # ours defaults to HEAD
```

C9-A ships the core function and tests only; the CLI verb lands in a later
slice once the shape has consumers. `inspect.next` integration (suggesting
prediction before risky integrations) is likewise future work.

## Limitations (Stated, Not Discovered)

- **A merge prediction is not a rebase transcript.** Rebase replays commits
  one at a time; conflicts can appear, disappear, or differ per step. The
  future rebase-chain predictor composes per-step predictions; until then,
  consumers must not present a single merge prediction as "what your rebase
  will do".
- The prediction ignores uncommitted state. A dirty working tree can still
  make the eventual real merge fail to start; `inspect` already covers that.
- Rename detection follows Git's merge defaults; pathological rename cases
  may predict differently than a tuned manual merge.
- `merge_base` in the output is one best ancestor, informational only.
- Requires Git >= 2.38 (`merge-tree --write-tree`). Older Git yields the
  `merge_tree_unsupported` structured error, never a fallback to the
  deprecated trial-merge mode.

## Non-Goals Restated

No execute. No undo. No automatic conflict resolution, ever, in this family —
prediction and guidance only (roadmap Stage 7 rule). No batch/chain
prediction, no branch-refresh wiring, no history-edit `drop`/reorder in this
checkpoint.

## C9-A Slice (Minimal Core)

- [x] `git/conflict_prediction.rs` in `super-git-core`: `predict_merge`
      resolving both revs, reporting merge base, running merge-tree, parsing
      with the pure parser above.
- [x] Output types in `model.rs` under
      `super-git.conflict-prediction.v0.1`.
- [x] Tests: clean merge, textual content conflict (stage shape asserted),
      modify/delete stage-absence shape, unknown rev, unrelated histories
      (`no_merge_base`), localized-fixture parser unit tests.
- [x] C9-B: `super-git predict merge --theirs <rev> [--ours <rev>]` CLI verb
      (ours defaults to HEAD; current repository only), JSON/human output,
      integration tests for both outcome classes and every structured error
      code reachable from the CLI.
- [ ] Inspect integration, batch/rebase-chain prediction: later slices.
