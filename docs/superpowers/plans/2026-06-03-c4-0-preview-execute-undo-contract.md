# C4-0 Preview Execute Undo Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define the write-side contract before implementing any Git mutation: `inspect -> preview -> execute -> undo`.

**Architecture:** `inspect` remains a read-only state snapshot. `preview` creates a declarative plan from a freshly validated action and state fingerprint. `execute` never trusts commands embedded in a plan; it re-validates the plan, recomputes state, regenerates Git commands from an internal allowlist, then returns an undo token for supported reversible writes.

**Tech Stack:** Rust workspace, serde JSON contracts, clap CLI, system `git`, SHA-256 fingerprints, integration tests with temporary Git repositories.

---

## C4-0 Scope

C4-0 is a docs-only contract checkpoint. It does not add `preview`, `execute`, or `undo` commands yet.

This checkpoint exists because the next stage enters destructive territory. The safety boundary must be explicit before the first write action lands.

## Lifecycle Contract

### 1. Inspect

`super-git inspect` describes the current repository posture only.

- It does not grant execution permission.
- `next.allowed` means preview candidate.
- `reference_command` is a documentation reference only.
- `inspect.risk_hint` is current-state risk only.

### 2. Preview

Future shape:

```bash
super-git preview stage-changes
```

`preview` reads fresh repository state, validates the requested action, and emits a plan:

```json
{
  "schema_version": "super-git.plan.v0.1",
  "plan_id": "sha256:<canonical-plan-hash>",
  "action": {
    "kind": "stage_changes",
    "scope": "all",
    "resolved_paths": [
      "file.txt",
      "new-file.txt"
    ]
  },
  "repository": "/abs/worktree/root",
  "state_fingerprint": {
    "schema_version": "super-git.fingerprint.v0.1",
    "repository": "/abs/worktree/root",
    "head_commit": "abc123",
    "operation": "none",
    "status_porcelain_v1_z_sha256": "sha256:<hash>",
    "staged_diff_sha256": "sha256:<hash>",
    "unstaged_diff_sha256": "sha256:<hash>",
    "untracked_content_sha256": "sha256:<hash>"
  },
  "preconditions": [
    { "code": "operation_none", "status": "passed" },
    { "code": "no_conflicts", "status": "passed" },
    { "code": "has_unstaged_or_untracked_changes", "status": "passed" }
  ],
  "risk": {
    "severity": "low",
    "reversibility": "reversible",
    "requires_human_confirmation": false
  },
  "effects": [
    "Stage unstaged and untracked changes in the current worktree."
  ],
  "reference_commands": [
    ["git", "add", "--all"]
  ],
  "undo_strategy": {
    "kind": "restore_index_snapshot",
    "requires_index_snapshot": true
  },
  "undo_preview": {
    "kind": "restore_index_snapshot",
    "available_after_execute": true
  }
}
```

The plan is a contract, not a script.

### 3. Execute

Future shape:

```bash
super-git execute --plan plan.json
super-git execute --plan -
```

`execute` must:

- Parse the plan and validate `schema_version`, `plan_id`, action kind, and action options.
- Recompute the current state fingerprint before any write.
- Fail with `precondition_mismatch` if the fingerprint or action preconditions changed.
- Ignore `reference_commands` when deciding what to run.
- Regenerate Git commands from an internal action allowlist.
- Return an `undo_token` only after the write succeeds.

`plan_id` is the SHA-256 hash of canonical JSON for the plan's execution contract, prefixed with a domain separator such as `super-git-plan-v0.1\n`.

The initial canonical JSON rule is deliberately small: UTF-8 JSON object, sorted object keys, no insignificant whitespace, and the same representation used by tests and production code.

Hash input includes:

- plan schema version
- action kind, scope, typed options, and resolved pathset
- repository root
- state fingerprint
- preconditions
- action risk
- undo strategy

Hash input excludes:

- `plan_id`
- `effects`
- `reference_commands`
- human-readable messages
- generated timestamps

### 4. Undo

Future shape:

```bash
super-git undo --token token.json
```

`undo_preview` is advisory. It tells the AI what kind of undo may become available.

`undo_strategy` is the machine-readable strategy that `execute` validates and includes in the plan hash.

`undo_token` is the recovery input produced by `execute` after a successful write. The token file is still untrusted input at undo time, so `undo` must validate repository scope, snapshot path, snapshot checksum, local registry provenance, and current state before restoring anything.

For `stage_changes`, the safest first undo model is index-snapshot based:

- Before execute, locate the worktree-specific Git index with `git rev-parse --git-path index` and snapshot that file.
- After execute, record the resulting index checksum.
- After execute, write a local registry record next to the snapshot so later undo can prove the token came from this repository's execute path.
- Undo restores the previous index only if the current index still matches the post-execute checksum.
- If the user or another tool changed the index after execute, undo fails with a structured error instead of clobbering new work.

## Trust Boundaries

1. `inspect` never writes and never grants permission.
2. `preview` may explain likely effects, but it still does not write.
3. `execute` does not trust plan commands, only validated action data and fresh state.
4. `undo_preview` is not an undo credential.
5. `undo_token` is scoped to one repository, one worktree, one action, and one observed before/after state.
6. A local undo registry record is provenance evidence, not authority by itself; it is cross-checked against the token and checksums.
7. Action risk belongs to preview/execute, not inspect.
8. Irreversible or weakly reversible actions require explicit human confirmation before execute.

## First Write Action: `stage_changes`

The first implemented write action is `stage_changes` because it is useful, narrow, and reversible with an index snapshot.

Initial scope:

- `kind = "stage_changes"`
- `scope = "all"`
- preview resolves and records the exact pathset that `scope = "all"` covers
- C4-A does not accept user pathspecs yet
- operation must be `none`
- conflict count must be `0`
- the index must not already contain staged changes
- there must be unstaged or untracked changes
- the plan fingerprint must match at execute time
- execute internally runs the allowlisted equivalent of `git add --all`

The first fingerprint implementation for `stage_changes` must lock the content being staged, not only the status shape:

- `status_porcelain_v1_z_sha256`: hash of `git status --porcelain=v1 -z`
- `staged_diff_sha256`: hash of `git diff --cached --binary --full-index`
- `unstaged_diff_sha256`: hash of `git diff --binary --full-index`
- `untracked_content_sha256`: hash of sorted untracked paths from `git ls-files --others --exclude-standard -z`, with each file's path, length, and bytes hash included

`scope = "all"` is an instruction to resolve the current unstaged/untracked pathset during preview. It is not an open-ended execute-time wildcard. If a file is added, removed, renamed, or modified after preview, execute must detect the changed fingerprint or pathset and fail before writing.

Pre-existing staged changes are allowed only if the undo token preserves the pre-execute index. Until index snapshot support is implemented, preview rejects staged-plus-unstaged states instead of returning a misleading reversible plan.

## Implementation Slices After C4-0

### C4-A: Preview model and `stage_changes` preview

**Files:**
- Create: `crates/super-git-core/src/git/fingerprint.rs`
- Create: `crates/super-git-core/src/git/preview.rs`
- Modify: `crates/super-git-core/src/model.rs`
- Modify: `crates/super-git-core/src/git/mod.rs`
- Modify: `crates/super-git-cli/src/args.rs`
- Modify: `crates/super-git-cli/src/main.rs`
- Modify: `crates/super-git-cli/src/output.rs`
- Test: `crates/super-git-cli/tests/preview.rs`

Acceptance:

- `super-git preview stage-changes` emits `{ ok: true, data: { schema_version, plan_id, action, state_fingerprint, preconditions, risk, effects, reference_commands, undo_strategy, undo_preview } }`.
- The command is read-only.
- Non-repo, clean repo, conflict repo, and operation-in-progress repo return structured errors or failed preconditions.
- `plan_id` is deterministic for the same canonical plan.
- Passing file pathspecs to `stage_changes` is rejected until a separate path-scoped design exists.

### C4-B: `execute --plan` for `stage_changes`

Acceptance:

- `execute` accepts `--plan <file>` and `--plan -`.
- It rejects malformed plans, unknown actions, bad hashes, and stale fingerprints before writing.
- It ignores `reference_commands` and uses the internal `stage_changes` allowlist.
- It returns an `undo_token` after successful staging.
- It writes a local undo registry record and rolls back the index if that record cannot be written.

### C4-C: `undo` for `stage_changes`

Acceptance:

- `undo` restores the previous index snapshot only when the current index still matches the token's post-execute checksum.
- It fails safely when the index changed after execute.
- It never alters working-tree file contents for `stage_changes` undo.

### C4-D: local undo registry provenance

Acceptance:

- `execute` writes a local undo registry record next to the index snapshot.
- `execute` rolls back the index if registry provenance cannot be written after the Git write.
- `undo` rejects tokens that have no matching local registry record or whose registry record was tampered with.
- The registry record is treated as untrusted input: missing, malformed, symlinked, unknown-field, token-mismatch, and token-hash-mismatch records fail safely.

## Non-goals

- No force push, reset, rebase, merge, cherry-pick, or worktree mutation in the first C4 implementation.
- No hidden plan store for the first execute path; the local undo registry stores only undo provenance.
- No raw shell command execution from JSON.
- No attempt to make inspect fingerprints strong enough for execution locks.

## Verification Checklist For Future C4 Tasks

- [ ] Preview is read-only in tests.
- [ ] Execute fails before writing when the plan is stale.
- [ ] Execute ignores tampered `reference_commands`.
- [ ] Plan hash changes when action data or preconditions change.
- [ ] Undo token is absent on failed execute.
- [ ] Execute registry write failure does not leave the index staged as a reported success.
- [ ] Undo rejects missing, symlinked, or tampered registry records.
- [ ] Undo refuses to clobber an index changed after execute.
- [ ] JSON envelope contract holds for success and failure.
- [ ] `--human` output stays secondary and compact.
