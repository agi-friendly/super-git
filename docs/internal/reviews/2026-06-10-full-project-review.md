# Full Project Review — super-git

> **Date:** 2026-06-10  **Scope:** entire repository (code, contracts, docs,
> tests, agent ergonomics) plus feature ideation.
> **Method:** 9 parallel review dimensions run as a multi-agent workflow
> (bugs-read, bugs-write, security, contract-consistency, cleanup, docs, tests,
> agent-ux, ideas), with adversarial verification per finding.

This is an internal review artifact, kept alongside `docs/internal/plans/` and
`docs/internal/idea/`. It is a snapshot, not an active contract.

## Verification status (read this first)

The verification pass was **interrupted by a session token limit** partway
through the bug-dimension verify step. As a result:

- **security** and **contract** findings were machine-verified: every item is
  marked `confirmed` by an independent adversarial verifier.
- **bugs-read** and **bugs-write** findings were **not** machine-verified; the
  lead (Fable) personally re-confirmed the high-severity ones against the code
  (marked ✅ below). The rest are finder-identified (◐) and credible but should
  get a verify pass before they are treated as ground truth.
- **cleanup / docs / tests / agent-ux** are design-level dimensions that did not
  need adversarial verification; they are finder-identified (◐).
- **ideas** is brainstorm output (see the end).

Legend: ✅ confirmed (verifier or lead) · ◐ finder-identified, plausible.

## Headline

super-git's safety architecture is genuinely strong: `inspect → preview →
execute → undo`, schema-versioned JSON contracts, `plan_id` re-derivation as the
anti-tamper spine, compare-and-swap ref moves, and provenance-checked undo. The
C8 history-edit family is a real differentiator. The issues below are almost all
**hardening and polish around a sound core**, not architectural faults. The two
things that genuinely bite today are (1) a handful of correctness bugs that can
break the core flow or misreport a destructive write, and (2) the documentation
having fallen a full feature-family behind the code.

## Top priorities (the must-fix shortlist)

| # | Severity | Theme | Finding | Where |
|---|----------|-------|---------|-------|
| 1 | ✅ HIGH | Bug | State fingerprint hashes external-diff output; `--no-ext-diff` missing → `diff.external`/difftastic users have **every `execute` fail** with a fingerprint mismatch (core flow permanently broken) | `git/fingerprint.rs:32,34` |
| 2 | ✅ HIGH | Bug | history_edit failures **after** the ref CAS (`read_status_signature`, `write_record_replace`) return a plain error: branch is already rewritten, undo token is lost, record stuck at `intent` → `undo` later refuses. Misreports a completed destructive rewrite as a failure | `git/execute_history_edit.rs:134,162` |
| 3 | ✅ MED(→HIGH) | Security | Incomplete env scrubbing: `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`VALUE_n` not stripped → ambient env injects `core.fsmonitor`/`core.hooksPath` = **arbitrary command execution** on every read/write; `GIT_NAMESPACE` silently retargets the history-edit CAS to the wrong ref | `git/command.rs:192` |
| 4 | ✅ MED | Security | Pointing super-git at an **untrusted repo path** (`inspect/status/wt list/repo add <path>`) runs that repo's `core.fsmonitor`; `worktree add` fires its `post-checkout` hook → command exec from read-only inspection of a hostile repo. Undocumented | `git/command.rs:176` |
| 5 | ✅ HIGH | Docs | **LICENSE mismatch**: `Cargo.toml` says `MIT`, root `LICENSE` is an unfilled Apache-2.0 template (`Copyright [yyyy] [name]`). Blocks adoption/publish | `Cargo.toml:10`, `LICENSE` |
| 6 | ✅ HIGH | Contract | README "Implemented today" + execute allowlist + safety-model.md omit the **entire history_edit family**; safety-model still says "Three Git write actions exist today" | `README.md:62`, `docs/safety-model.md:97` |
| 7 | ◐ MED | Bug | stage_changes execute/undo rollback paths **leak `.git/index.lock`** on several error branches → repo blocked for all git writes until manual unlink | `git/execute.rs:549`, `git/undo.rs:131` |
| 8 | ◐ MED | Bug | Orphan `intent` execution records (deterministic path, `create_new`) **permanently block re-execution** of a still-valid plan with a raw `File exists` io error | `git/execute_history_edit.rs:104` + worktree create/remove |
| 9 | ◐ HIGH | Agent-UX | JSON error envelope has **no machine-readable `code` field**; codes are buried in prose `causes[]`. Agents must regex English to branch retry/abort | `cli/output.rs:38` |
| 10 | ◐ HIGH | Agent-UX | Confirmation phrase is **unconstructible from CLI output**: plan's `human_prompt` differs from the enforced phrase, mismatch error withholds the template, worktree_remove phrase is in no public doc | `git/execute.rs:240`, `cli/output.rs` |
| 11 | ◐ HIGH | Agent-UX | `stage_changes` plans **self-invalidate** when the agent saves `plan.json` in the repo cwd (it's untracked content the fingerprint hashes). The most natural flow always fails | `git/fingerprint.rs:61` |
| 12 | ◐ HIGH | Test | The **super-git binary under test inherits the developer's real `~/.gitconfig`** (only history-edit suites pin repo-local config). `status.showUntrackedFiles=no`/`core.fsmonitor` can break suites or mask bugs | `cli/tests/*.rs` |
| 13 | ✅ MED | Bug | history_edit instruction resolution caps oids at **40 chars** (`history_edit.rs:494`) but undo accepts 64 → history-edit **entirely unusable in SHA-256 repos** | `git/history_edit.rs:494` |
| 14 | ◐ HIGH | Cleanup | Fold grouping implemented **twice** (`build_program` preview vs `build_groups` execute); plan_id + tree post-verify cannot detect a message/author divergence between them | `git/execute_history_edit.rs:456` |

---

## Theme A — Correctness bugs that can break the core flow

### A1. External-diff leakage in the state fingerprint ✅ HIGH
`read_state_fingerprint` runs `git diff --cached --binary --full-index` and
`git diff --binary --full-index` (`fingerprint.rs:29-34`) **without
`--no-ext-diff`**. With `diff.external` configured (the documented difftastic
setup) or `GIT_EXTERNAL_DIFF` in the ambient env, git replaces the diff stream
with the driver's output, which typically embeds a per-invocation temp path.
preview computes hash A, execute recomputes hash B → **every execute fails** with
a `state_fingerprint` mismatch. Conversely a content-independent driver lets a
staged change slip past revalidation, defeating the anti-tamper check.
**Fix:** add `--no-ext-diff --no-textconv` to both invocations (or use
`git diff-index`/`diff-files` plumbing), and `env_remove("GIT_EXTERNAL_DIFF")`.

### A2. history_edit loses the undo token on post-CAS failure ✅ HIGH
After `compare_and_swap_ref` moves the branch and `post_verify` passes, two steps
use a bare `?`: `read_status_signature` (`execute_history_edit.rs:134`) and
`write_record_replace` (`:162`). A failure there returns a generic Io/Git error:
no `ExecutePartialFailure`, no rollback, no undo token, and the record stays at
`status:"intent"` so `undo_history_edit` later refuses with
`execution_record_incomplete`. The highest-risk action is the **only** write
family that doesn't wrap its post-write window in `partial_failure`. An agent
concludes nothing happened while published history was actually rewritten and is
now un-undoable through the tool.
**Fix:** treat any post-CAS failure like the worktree paths — roll the ref back,
or return `ExecutePartialFailure` carrying `record_path`, `previous_tip`,
`new_tip`.

### A3. SHA-256 repos can't use history-edit ✅ MED
`resolve_range_commit` rejects inputs longer than 40 hex chars
(`history_edit.rs:494`); SHA-256 repos use 64-char oids, which the scan itself
emits. Echoing them back (the natural round-trip) makes every instruction
resolve to `None` → `instructions_unknown_commit`. `undo_history_edit.rs:306`
already allows 64. **Fix:** raise the cap to 64 (or derive from resolved oid
length).

### A4. Empty-author commits make an "executable" plan always fail ◐ LOW
`commit_tree` exports `GIT_AUTHOR_NAME=""` verbatim for commits with an empty
author ident (legal, producible by importers). Preview has no block for it, so
the plan is `executable`, but `git commit-tree` then errors "empty ident name
not allowed" forever. **Fix:** detect empty author name/email during scan and
emit a hard block (`author_identity_unreadable`).

### A5. `git add` pathspec magic on resolved paths ◐ LOW
Resolved paths are appended after `--` to `git add --all`, but git still applies
pathspec semantics: a filename starting with `:` becomes pathspec magic
(`:foo`→pattern, `:(exclude)src`→exclude), glob metachars are wildmatched.
`validate_relative_path` doesn't reject these. **Fix:** emit each path as
`:(literal)<path>`, or reject `:`-prefixed paths at preview.

---

## Theme B — Security hardening

### B1. Incomplete Git environment scrubbing ✅ MED (argue HIGH)
`base_command` strips `GIT_DIR/GIT_WORK_TREE/GIT_COMMON_DIR/GIT_INDEX_FILE/
GIT_PREFIX` but **not** the config-injection family
(`GIT_CONFIG_COUNT` + `GIT_CONFIG_KEY_n`/`VALUE_n`, `GIT_CONFIG_GLOBAL/SYSTEM`),
`GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_NAMESPACE`,
`GIT_CEILING_DIRECTORIES`. The in-code comment claims "bind to `-C`, not ambient
env" — the missing entries directly contradict it. Verified: ambient
`GIT_CONFIG_*` setting `core.fsmonitor` yields **arbitrary command execution** on
inspect/status/execute (`GIT_OPTIONAL_LOCKS=0` doesn't suppress it). Worse for
safety: `GIT_NAMESPACE` retargets the history-edit `update-ref refs/heads/X` CAS
to a different namespace than the plan bound to, so the CAS guarantee operates on
the wrong ref. **Fix:** extend the `env_remove` loop.

### B2. Hostile-repo command execution, undocumented ✅ MED
`inspect/status/wt list/repo add <path>` run git inside a user-supplied path; a
malicious repo's `core.fsmonitor` runs on even read-only commands, and
`worktree add` fires the repo's `post-checkout` hook. A *safety* tool achieves
command exec from read-only inspection of a hostile repo, with no warning.
**Fix:** for read paths run with `-c core.fsmonitor=` (and consider
`-c core.hooksPath=/dev/null` where it doesn't break intent); document the limit
in `safety-model.md`.

### B3. Option-like branch name into `worktree add` ✅ LOW
`trusted_git_args` builds `git worktree add -q <path> <checkout_arg>` with no
`--end-of-options`. A branch named `--force` (creatable via `update-ref`,
resolvable) reaches `worktree add` as an injected option. Impact is bounded
(single trailing token, must be a real ref, path pre-validated), so this is
hardening. **Fix:** insert `--end-of-options` before positionals across
worktree/rev-parse/update-ref calls that take artifact-derived refs.

### B4. Untagged `ExecuteUndoToken` enum — audited, not exploitable ✅ INFO
The untrusted parse path dispatches on the explicit `kind` field (not untagged
deser), all variants have `deny_unknown_fields` with disjoint required fields,
and execution-relevant fields are re-validated against on-disk records + live
repo. No change required; optionally make the enum internally tagged on `kind`.

---

## Theme C — Resource leaks & partial-state handling

- **C1 ◐ MED** `.git/index.lock` leaks: `rollback_index_after_registry_failure`
  restore branch (`execute.rs:549,561-572`) and `undo_index_token`
  `hash_index`/`restore_index` error paths (`undo.rs:131,268-279`) return without
  unlinking the lock. The sibling `remove_index_after_failed_execute` cleans up;
  these don't. Result: repo blocked for all git writes until manual unlink.
  **Fix:** guard the restore/hash section so the lock is removed on any error.
- **C2 ◐ MED** Orphan `intent` records block re-execution (all three write
  families). The CAS-failure branch removes the record; the post_verify-failure
  rollback branch does **not**. Re-running the still-valid plan dies at
  `write_record_create_new` with raw `File exists (os error 17)`.
  **Fix:** remove the record after successful rollback; map `AlreadyExists` to a
  contract error (`execution_already_attempted`), or allow replacing an `intent`
  record.
- **C3 ◐ LOW** `worktree_remove` `read_branch_oid` failure after the intent write
  isn't wrapped in `partial_failure` (`execute_worktree_remove.rs:48-51`) — move
  the read above the intent write (it's read-only) or wrap it.
- **C4 ◐ LOW** worktree undo/remove TOCTOU: ignored files created between the
  cleanliness check and `git worktree remove` are deleted despite the undo's
  "refuses if dirty" promise. **Fix:** re-check immediately before removal;
  document the residual race.

---

## Theme D — Cross-platform, encoding & config-sensitivity

These are mostly Linux/Windows/non-ASCII exposures (the suite runs on macOS).

- **D1 ◐ MED** Conflict path lists return **C-quoted/escaped** names
  (`UU "caf\303\251.txt"`) for non-ASCII paths, because `status --porcelain=v1`
  (non-`-z`) is parsed and `line[3..]` stored verbatim. Three copies:
  `state.rs:282`, `history_edit.rs:806`, `worktree_remove.rs:239`. The
  machine-readable conflict list is unusable exactly for paths that need machine
  handling. **Fix:** use `--porcelain=v1 -z` + NUL split, or unquote.
- **D2 ◐ LOW** Lossy UTF-8 (`from_utf8_lossy`) of git stdout mangles non-UTF-8
  paths into `U+FFFD` → nonexistent paths (`command.rs:233`, Linux exposure).
  **Fix:** capture raw bytes + `OsString::from_vec` for path-producing commands.
- **D3 ◐ LOW** worktree-list parser uses non-`-z` porcelain; a **newline in a
  worktree path** truncates the entry (`worktree.rs:16`). **Fix:** `-z`.
- **D4 ◐ LOW** Case-insensitive collision check uses Unicode `to_lowercase`, not
  filesystem semantics: over-blocks on case-sensitive Linux, misses NFC/NFD
  collisions on macOS APFS (`worktree_plan.rs:193`).
- **D5 ◐ LOW** `read_working_tree` inherits `status.showUntrackedFiles` config
  (`state.rs:265`): a repo/user `= no` makes inspect report `clean`/`untracked:0`
  while untracked files exist, and stage-changes preview then fails its
  precondition → agent thinks there's nothing to commit. `worktree_remove.rs`
  already pins `--untracked-files=all`; the read side is internally inconsistent.
  **Fix:** pin the mode in `read_working_tree` and the fingerprint status hash.
- **D6 ◐ LOW** `repository_name` strips `.git` from non-bare worktree dir names
  (`store.rs:740`): `/work/service.git` registers as `service`; a dir named
  `.git` yields the empty name. **Fix:** strip only for the bare git_common_dir
  case.
- **D7 ◐ LOW** Untracked-content hashing: a file vanishing between `ls-files` and
  read yields a path-less ENOENT (`fingerprint.rs:107`); whole files are buffered
  in memory (20 GB untracked dataset → OOM on every preview/execute). **Fix:**
  wrap the error with the path; stream into the hasher.
- **D8 ◐ INFO** Commit subject = first physical line of `%B`
  (`history_edit.rs:728`), not git's `%s` (first paragraph folded) → mismatches
  `git log --oneline` for wrapped subjects.

---

## Theme E — Contract & documentation drift (all ✅ confirmed)

The dominant theme: **C8 history-edit shipped without updating the public
contract docs.** AGENTS.md:80-81 mandates updating safety-model/architecture/
roadmap in the same slice as a safety-contract change; that didn't happen.

- **E1** README "Implemented today" omits history-edit; the execute allowlist
  sentence ("`stage_changes`, executable `worktree_create`, confirmed
  `worktree_remove`") is now factually false; undo section omits
  `restore_branch_tip_snapshot`. → agents fall back to `git rebase -i`.
- **E2** `safety-model.md` (the declared "active safety contract") still says
  "Three Git write actions exist today"; no history-edit, no fresh-binding model,
  no confirmation-beyond-worktree-remove, no branch-tip undo.
- **E3** `architecture.md` command list + Preview/Execute/Undo contract predate
  history-edit; no pointer to the C8-0 plan (unlike C6/C7).
- **E4** `AGENTS.md` describes confirmation as exclusive to *non-undoable*
  actions — published history-edit is confirmation-gated **and** undoable,
  breaking the taxonomy; undo "before changing the index or removing worktree
  state" omits branch-ref moves.
- **E5** `command-reference.md:354` promises history-edit undo "verifies that no
  other ref changed" — the code deliberately does **not** (C8-D review removed
  that check). Also "still reachable locally" should be "still exists in the
  object store" (the old tip is unreachable after execute). The C8-0 acceptance
  box for this is checked but unimplemented.
- **E6** C8-0 step 17 rollback reporting (`failed_rolled_back`/`failed_partial`
  with record path + tips) is **not implemented**; a successful rollback
  re-raises the original error with no marker; no test exercises the rollback
  path though its C8-C box is checked.
- **E7** C8-0 status banner still says "No history_edit preview, execute, or undo
  behavior exists yet" while every C8-A…E box is checked — self-contradictory.
- **E8** roadmap "Current Position" says "two undoable write-side flows, one
  destructive" — contradicts its own Stage 6 implemented list.
- **E9** Per-version execution-block divergence (`super_git_execute_required`
  v0.2 vs `execute_supported`/`future_execute_eligibility` v0.3 vs
  `requires_confirmation_artifact` v0.4) is undocumented; C8-0's "unifies" claim
  is misleading (old versions keep their fields).
- **E10** command-reference overstates that only instruction input can fail
  `preview history-edit` (unresolvable `--base`, unborn HEAD, outside-worktree
  also fail with `ok:false`).
- **E11** Stale execute error: "confirmation artifacts are supported only for
  destructive worktree_remove plans" — published history-edit also takes one
  (`execute.rs:165`). (Also flagged as bugs-write#10.)
- **E12** No history-edit walkthrough in `getting-started.md` (only stage-changes).
- **E13** LICENSE/Cargo.toml mismatch (see top shortlist #5).
- **E14** Normative contract content (confirmation artifact spec, plan v0.4
  instruction format) lives **only** in `docs/internal/`, violating
  `docs/README.md:35`'s own public/internal rule.
- **E15** No CI workflow despite a "keep every commit green" culture; the 11 CLI
  suites are CI-ready. Suggest `.github/workflows/ci.yml` (fmt/clippy/test +
  `git diff --check`) on a ubuntu/macos/windows matrix.
- **E16** `docs/README.md` internal-plans list is hand-curated and stale (3
  plans missing, `internal/idea/` unlisted).

---

## Theme F — Agent ergonomics

The product is "AI-first", so these are first-class, not polish.

- **F1 ◐ HIGH** No machine-readable error `code` field; codes live in prose
  `causes[]`. Add `error.{code, kind, details?, recovery?}` via a
  `SuperGitError::code()` method (each struct variant already owns a `code`).
- **F2 ◐ HIGH** Confirmation phrase unconstructible from CLI: plan's
  `human_prompt` ≠ enforced phrase; mismatch error withholds the template;
  worktree_remove phrase in no public doc. Add `confirmation.required_phrase`
  (or `phrase_template` + values) to the plan; document both templates.
- **F3 ◐ MED** Confirmation artifact shape learnable only via `deny_unknown_fields`
  error breadcrumbs (one field per failed *destructive* execute). Emit a prefilled
  `confirmation_template` object in the plan; populate `suggested_super_git_command`
  for `preview_only` worktree_remove plans (history-edit already does).
- **F4 ◐ HIGH** stage_changes plans self-invalidate when `plan.json` is saved in
  the repo (untracked content the fingerprint hashes). Add a targeted hint when
  only `untracked_content_sha256` differs; advertise the
  `preview … | execute --plan -` pipe idiom in help.
- **F5 ◐ MED** Fingerprint mismatch serializes two full fingerprint JSON docs into
  one prose string, no changed-field summary, no "rerun preview" hint. Route
  through `structured_error_details` with `{changed:[…], recovery:"rerun_preview"}`.
- **F6 ◐ MED** `inspect.next` never surfaces worktree-create/remove or
  history-edit — capability discovery dead-ends at the C3-era action set. An
  agent asked to "squash these 3 commits" runs `inspect`, sees nothing, reaches
  for `git rebase -i`. Add state-conditional preview candidates.
- **F7 ◐ MED** `inspect.next` proposes actions super-git can't perform
  (`commit`/`push` with `execution_contract:preview_required` but no such preview
  subcommand) and embeds raw `reference_command`s under `raw_git_allowed:false` —
  a contradictory dead end. Add `supported_by_super_git` + `suggested_super_git_command`
  to `NextAction`.
- **F8 ◐ MED** Execution/confirmation contract diverges across v0.1–v0.4 (field
  names, status vocabularies, three overlapping confirmation flags). Converge on
  one shared execution core in the next schema rev; document a per-version
  decision table meanwhile.
- **F9 ◐ MED** history-edit instructions JSON schema undiscoverable from the CLI
  (survey says `<instructions-file>` but never shows the document shape; a bare
  array yields "expected a string"). Embed an `instructions_template` (prefilled
  `pick` for every range commit, in order) in survey plans — solves shape,
  completeness, and ordering at once.
- **F10 ◐ LOW** Every failure exits `1`; no taxonomy. Adopt a small stable set
  (usage / stale-retryable / invalid-tampered / partial-failure).
- **F11 ◐ LOW** Misleading codes: `confirmation_acknowledgement_missing` for a
  *present-but-wrong* method; `unsupported_schema_version` for null/non-plan
  input; duplicated `causes` from `SuperGitError::Json`.
- **F12 ◐ INFO** The `--plan -` / `--confirmation -` / `--instructions -` /
  `--token -` stdin conventions and the envelope-accepting pipe idioms
  (`preview … | execute --plan -`) are excellent but **undocumented**; the
  both-stdin rejection is a codeless `anyhow::bail`; `--human` flips errors from
  stdout to stderr (a "no output" trap).

---

## Theme G — Code quality / consolidation backlog (◐, all verified by finder)

The duplication backlog is real and several copies have **already drifted**.
Recommended shared modules + a green-keeping migration order:

1. **`git/hash.rs`** — `hex(bytes)` + `sha256_prefixed(bytes)` (10 copies;
   `execute_worktree_remove.rs:529` already uses a different impl) and
   `sha256_with_domain<T>` + `sort_json_value` canonical JSON (3 copies; **note:**
   `preview.rs` v0.1 deliberately does *not* sort keys — keep it with a WHY
   comment or migrate in a fixture-updating commit, since it changes in-flight
   v0.1 plan_ids).
2. **`git/checks.rs`** — `plan::{invalid,mismatch,ensure_match}` over
   `ExecutePlanInvalid`/`ExecutePreconditionMismatch` and `token::{…}` over the
   undo error family (8 copies, signature already drifting); plus
   `ensure_path_match`/`ensure_option_match`/`validate_absolute_clean_path`.
3. **`git/records.rs`** — `execution_record_path(git_common_dir, id)`,
   `write_create_new<T>`, `write_replace<T>` (3 verbatim copies) and
   `validate_execution_record_path(git_common_dir, path)` (a **security check on
   untrusted input** duplicated verbatim across the two undo modules — silent
   divergence here is a safety problem).
4. **`git/confirm.rs`** — `validate_confirmation_common(c, expectations)` (8
   identical destructive-confirmation checks duplicated between worktree_remove
   and history_edit, same error codes; requires unifying the two `*Confirmation`
   model types first).
5. **`Operation: Display`** in `model.rs` (matching the serde kebab-case strings)
   → delete two `operation_name` copies.
6. **`git/status.rs`** ← move `is_conflict`/`is_change` (tripled).
7. **`git/repository.rs`** ← `worktree_root`/`git_common_dir`/`read_ref_oid`
   helpers (note: `store.rs`'s copy **canonicalizes** — keep it distinct, rename
   `canonical_git_common_dir`).
8. **Highest-value:** unify `build_program` (preview) and `build_groups`
   (execute) fold grouping. They're equivalent today but `plan_id` + tree
   post-verify cannot catch a future divergence in message/author attribution on
   a history rewrite. Extract `fold_groups(instructions, commits)` in
   `history_edit.rs` and have execute consume it (it already runs
   `preview_history_edit` for the fresh binding — expose the program through that
   path).
9. **NEW** Dead conditional in `undo.rs:68` (`token_from_data` — both branches
   call `parse_token_value`); the implied schema-version check never happens.
10. **NEW** Three `same_path` definitions, two semantics under one name
    (`execute_worktree_remove.rs:510` is literal-only; the others canonicalize) →
    consolidate to the canonicalizing version (changes worktree_remove behavior
    on symlinked tmp paths — flag in the commit).

Migration discipline: one file per commit, integration suites pin error JSON and
`plan_id`/digest values so each commit is self-verifying; do the
`build_program`/`build_groups` unification last since it touches execution
semantics.

---

## Theme H — Test coverage gaps (◐)

- **H1 HIGH** super-git binary inherits the developer's real global/system
  gitconfig (only history-edit suites pin repo-local config). Set
  `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` to empty temp files on every
  `super_git()` helper; add a hostile-config canary test.
- **H2 HIGH** history_edit `rollback()` (post-verify-failure CAS rollback) and
  `ExecuteRollbackFailed` have **zero** coverage — the one mechanism that
  prevents a half-applied destructive rewrite. Add direct unit tests for both the
  rollback-succeeds and rollback-also-fails cases.
- **H3 MED** `ExecutePartialFailure` (worktree create/remove) untested at any
  level despite carrying an agent-facing recovery contract.
- **H4 MED** `HISTORY_EDIT_RANGE_CAP` boundary: only cap+1 (101) tested,
  exactly-100 never (`>` vs `>=`); no CLI-level `range_too_large` envelope test.
- **H5 MED** Unborn-HEAD (empty `git init`) — a common agent bootstrap state —
  handled in code, untested for every command.
- **H6 MED** history-edit has no bare-repo test; failure is an unpinned raw git
  error (the bare-primary layout this tool promotes).
- **H7 MED** No filenames with spaces/unicode/quotes or rename entries anywhere
  in the stage_changes pipeline.
- **H8 MED** Execute-side `index.lock` contention untested (undo side is).
- **H9 LOW** history-edit **execution-record** content tampering untested (only
  token-side tamper + deletion covered); the record-vs-token equality check is
  never exercised with mismatching bytes.
- **H10 LOW** `config_path` test depends on the developer's real HOME; no
  "didn't create a real home dir" assertion.
- **H11 LOW** Oversized-range fixture spawns 101 `git commit` subprocesses (~1-3s,
  the slowest unit test) — use one `git fast-import`/`commit-tree` chain.
- **H12 LOW** macOS case-insensitive FS path-case behavior only tested as string
  comparison; no real divergent-casing undo test.

---

## Feature ideas — "let AI handle Git better than humans with IDEs"

Grounded in the existing safety machinery (plan_id, fresh-binding, CAS, undo
provenance) and the roadmap. Ranked by leverage.

| # | Idea | Impact | Effort | Fit | One-line |
|---|------|--------|--------|-----|----------|
| 1 | **Conflict prediction** `preview conflicts --ours --theirs` | high | med | Stage 7 (the core) | `git merge-tree --write-tree` read-only sim → per-file predicted conflicts + contributing commits; no execute/undo (pure inspect) |
| 2 | **history-edit drop/reorder** | game-changer | large | Stage 7→6 | The #1 demanded op; gate with idea-1 prediction; generalize tree-identity post-verify to "simulated tree id"; reuses CAS + confirmation + branch-tip undo |
| 3 | **safe branch refresh** `preview branch-refresh --onto --branches` | high | med | Stage 7 (already assigned) | Checkout-free batch fast-forward via existing/temp worktrees; plan names which worktree's files change; batch-CAS undo |
| 4 | **publish** `preview publish --branch` | high | med | NEW (pairs Stage 6) | Frozen `--force-with-lease=<ref>:<expected_remote_tip> --force-if-includes`; confirmation-gated when it rewrites remote; honest `not_available` undo + recovery hint |
| 5 | **absorb** (autofixup) `preview absorb --base` | game-changer | large | NEW | Route staged hunks into the commit that introduced them via blame; invariant `expected_final_tree == tree(HEAD + staged)`; two-part undo (branch tip + index snapshot) |
| 6 | **commit split** (path-MVP) | high | med | Stage 6 (deferred split) | `instructions v0.2` `{split, parts:[{subject, paths}]}`; tree-only partition, last part's tree == original → provably conflict-free; same execute/undo as history-edit |
| 7 | **checkpoint/restore** | high | large | NEW | Safer than stash: snapshot index+worktree+untracked to `refs/super-git/checkpoint/*` (no file touch); restore auto-saves a counter-checkpoint → symmetric undo |
| 8 | **inspect timeline** | high | med | Stage 9 (pull earlier) | Fuse per-ref reflog + HEAD reflog + in-progress markers + super-git execution records by plan_id → `recovery_candidates` routed to the existing undo machinery |
| 9 | **lost-commit rescue** `inspect lost-commits` + `preview rescue` | med | small | NEW | `fsck --unreachable` + expiring reflog → structured; rescue = pure ref creation, undo = CAS ref delete; low-risk Stage-8 one-shot candidate |
| 10 | **graduated one-shot** `super-git run stage-changes` | high | small | Stage 8 | preview→validate→execute in one call for `reversibility:automatic && !requires_human_confirmation` actions; add `error.next[]` machine-actionable payloads |
| 11 | **commit-message lint** `lint commit-message` | med | small | NEW | Repo `.super-git/commit-policy.json`; history-edit preview auto-lints reword/squash results into plan `warnings`; warn-tier by default, `--strict` → blocked |
| 12 | **token-budget structured diff** `inspect diff --budget-bytes` | high | med | Stage 9 (shared good) | Rename detection + file classification + graduated degradation (hunks→headers→stat) → cuts the context-token cost of every other flow |

The strongest near-term cluster: **idea 1 (conflict prediction) unlocks ideas 2,
3** and is already the roadmap's Stage 7 centerpiece. **Ideas 5 (absorb), 6
(split), 9 (rescue)** all slot onto the existing history-edit plumbing with
honest undo stories. **Idea 10 (one-shot) + F1/F2/F3/F9 (machine-readable errors
and prefilled templates)** together would most improve weak-agent success rates.

---

## Suggested action plan

**Now (correctness & safety, small diffs):**
1. A1 `--no-ext-diff` on fingerprint diffs (+ env_remove `GIT_EXTERNAL_DIFF`).
2. A2 history_edit post-CAS failure handling (rollback or partial-failure).
3. B1 extend env scrubbing (config-injection family, namespace, object dirs).
4. C1/C2 index.lock leak guards + orphan-intent-record handling.
5. A3 oid cap → 64.
6. E13 resolve the LICENSE/Cargo.toml mismatch + add README License section.

**Soon (contract truth & agent UX):**
7. E1–E12 doc sync: README, safety-model, architecture, AGENTS, roadmap,
   command-reference, getting-started, C8-0 banner; graduate the confirmation +
   instruction specs to public docs (E14).
8. F1 structured error `code` field; F2/F3/F9 prefilled phrase/confirmation/
   instructions templates in plans; F4 stage_changes-in-repo hint.
9. B2 hostile-repo hardening + documented limitation.
10. H1 pin gitconfig in all test suites; H2 rollback-path tests.

**Then (quality & capability):**
11. Theme G consolidation (hash/checks/records/confirm modules; unify
    build_program/build_groups) — one green commit per file.
12. Begin Stage 7 conflict prediction (idea 1), which unlocks drop/reorder and
    branch-refresh.

**Process:**
13. E15 add CI (fmt/clippy/test/diff-check) on a 3-OS matrix.

---

*Generated from a 9-dimension multi-agent review. Security and contract findings
were adversarially verified (`confirmed`); bug findings marked ✅ were
re-confirmed against the code by the lead; remaining items (◐) are
finder-identified and warrant a verify pass before being treated as ground
truth.*
