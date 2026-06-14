# Command Reference

`super-git` defaults to JSON output. Add `--human` for terminal-friendly text.

```bash
super-git inspect
super-git --human inspect
```

## JSON Envelope

Success:

```json
{ "ok": true, "data": {} }
```

Failure:

```json
{ "ok": false, "error": { "message": "...", "causes": [] } }
```

Parse errors, runtime errors, and command errors should all respect this
contract unless the user explicitly asks for `--help` or `--version`.

Unless a JSON example is explicitly labeled as a full envelope, examples below
show the `data` payload inside `{ "ok": true, "data": ... }`.

## `doctor`

Checks whether the local environment can run `super-git`.

```bash
super-git doctor
```

Reports the system Git version, OS, architecture, and config path.

## `config path` / `config show` / `config validate`

Reports the resolved `super-git` app home and config file.

```bash
super-git config path
super-git config show
super-git config validate
```

`config path` returns the app home, resolution source, and `config.json` path.
It does not create the config file.

`config show` returns the same location plus the currently loaded v1 config. If
no config file exists yet, the command returns the empty default config without
creating a file.

```json
{
  "location": {
    "home": "/tmp/super-git-home",
    "source": "env:SUPER_GIT_HOME",
    "config_file": "/tmp/super-git-home/config.json"
  },
  "config": {
    "schema_version": 1,
    "settings": {
      "worktree": {
        "parent_template": "{main_path}.worktrees",
        "name_template": "{repo_name}__{ref_slug}",
        "ref_slug_algorithm": "path_safe_v1"
      }
    },
    "repositories": []
  }
}
```

Existing v0 files shaped like `{ "repositories": [...] }` are migrated in memory.
The next write saves the current v1 shape. Unknown future schema versions fail
with a JSON error envelope instead of being partially interpreted.
Legacy repository paths that no longer resolve to Git repositories are skipped
during migration because they cannot be assigned a worktree-family identity.

`config validate` validates the loaded config without writing it. If the config
file is missing, it validates the default in-memory v1 config and does not create
`config.json`. Validation covers both worktree template settings and saved
repository registry shape. Registry entries must have absolute path fields,
valid case-preserving `sha256:<git-common-dir>` identities, unique ids, and a
`kind`/`main_worktree` combination that matches whether the family has a primary
worktree.

Invalid user-editable settings are reported as a successful validation payload,
not as a command failure:

```json
{
  "location": {
    "home": "/tmp/super-git-home",
    "source": "env:SUPER_GIT_HOME",
    "config_file": "/tmp/super-git-home/config.json"
  },
  "valid": false,
  "issues": [
    {
      "field": "settings.worktree.name_template",
      "code": "unknown_template_variable",
      "message": "unknown template variable {branch}"
    }
  ]
}
```

Set `SUPER_GIT_HOME` to isolate tests, CI, dogfooding, or subagent work from the
real user config. Without it, `super-git` uses the OS-specific config location.

## `config set-worktree-template`

Updates worktree path template settings in the global config.

```bash
super-git config set-worktree-template \
  --parent-template '{main_path}.worktrees' \
  --name-template '{repo_name}__{ref_slug}' \
  --ref-slug-algorithm path_safe_v1
```

At least one option is required. Omitted fields are preserved.

Successful updates write the v1 config shape and return the updated config data:

```json
{
  "location": {
    "home": "/tmp/super-git-home",
    "source": "env:SUPER_GIT_HOME",
    "config_file": "/tmp/super-git-home/config.json"
  },
  "changed": true,
  "config": {
    "schema_version": 1,
    "settings": {
      "worktree": {
        "parent_template": "{main_path}.worktrees",
        "name_template": "{repo_name}__{ref_slug}",
        "ref_slug_algorithm": "path_safe_v1"
      }
    },
    "repositories": []
  },
  "validation": {
    "valid": true,
    "issues": []
  }
}
```

Validation rules:

- Template variables use braces, such as `{ref_slug}`. Shell-style `$REF` or
  `${REF}` syntax is rejected.
- Supported variables are `{main_path}`, `{repo_name}`, and `{ref_slug}`.
- `parent_template` must contain `{main_path}` exactly once and must not contain
  `{ref_slug}` or a literal `..` path component.
- `name_template` must contain `{ref_slug}` exactly once and must not contain
  `{main_path}`, `/`, or `\`.
- `ref_slug_algorithm` currently supports only `path_safe_v1`.

Invalid updates fail with `{ "ok": false, "error": ... }` and do not rewrite the
existing config file.

## `inspect [path]`

Returns the AI-first repository safety snapshot.

```bash
super-git inspect
super-git inspect /path/to/repo/or/subdir
```

The snapshot includes:

- repository root
- worktree context
- HEAD state
- upstream ahead/behind
- working-tree summary
- in-progress Git operation
- warnings and current-state risk hint
- guarded next-action candidates

`inspect` is read-only. Its next-action fields are not permission to execute raw
Git commands. Use `preview` before any write.

When the repository state allows it, `next.allowed` also lists the
preview-gated write flows (`worktree_create`, `history_edit`) as preview
candidates. Their `reference_command` points at the matching `super-git
preview` entrypoint with placeholder arguments such as `<ref>` and `<base>`:
the placeholders must be replaced before the command can run, which keeps the
hint documentation-only. When a flow's preview would refuse (in-progress
operation, conflicts, detached or unborn HEAD), `next.blocked` carries the
reason instead.

## `preview stage-changes`

Builds a read-only plan for staging current unstaged and untracked changes.

```bash
super-git preview stage-changes > /tmp/super-git-plan.json
```

The plan is a contract, not a script. `reference_commands` are documentation
references only.

## `preview worktree-create`

Builds a read-only `super-git.plan.v0.2` plan for creating one linked worktree.

```bash
super-git preview worktree-create --ref <branch-or-tag-or-commit>
super-git preview worktree-create --repo <id-or-name-or-path> --ref <branch-or-tag-or-commit>
```

Supported source refs are existing local branches, tags, and commit hashes.
Remote-tracking branches are recognized but blocked until an explicit
local-branch policy exists. Ambiguous refs, occupied local branches, and target
path collisions also return blocked plans instead of letting Git fail later.

`preview worktree-create` does not create directories, worktrees, config files,
or Git worktree metadata. Unblocked plans use
`execution.status: "executable"` and must still pass `execute --plan`
re-validation before any write occurs.

## `preview worktree-remove`

Builds a read-only `super-git.plan.v0.3` plan for removing one existing linked
worktree.

```bash
super-git preview worktree-remove --worktree <absolute-linked-worktree-path>
```

The first implementation intentionally accepts only an exact absolute path that
matches one `git worktree list --porcelain` entry. There is no `--current`, no
`--force`, no branch deletion, and no automatic undo.

If the path does not exactly match a worktree-list entry, no target-specific
plan is emitted; the command fails with `{ ok: false, error }` instead.

Clean linked worktrees return `execution.status: "preview_only"` with
`execute_supported: true`. Blocked targets return `execution.status:
"blocked"` with structured hard-block reasons. In both cases the plan is
read-only and includes high-risk metadata, explicit confirmation requirements,
`undo_strategy.kind: "not_available"`, recovery hints, and documentation-only
`reference_commands`.

## `preview history-edit`

Builds a read-only `super-git.plan.v0.5` plan for editing commit history on the
branch checked out in the current worktree. The op set is `pick`, `reword`,
`squash`, `fixup`, and `drop`; reorder is expressed by changing the order of
the instruction `items` array itself. `pick`/`reword`/`squash`/`fixup` and
clean reorder plans preserve the final tree; `drop` removes a commit's patch
from the final history and is gated by conflict prediction (below).

```bash
super-git preview history-edit --base <ref>
super-git preview history-edit --base <ref> --instructions <file|->
```

`--base` names the last commit that stays untouched; the editable range is
`base..HEAD`. Without `--instructions`, the command returns a read-only survey
(`execution.status: "survey"`) whose `range.commits` array is the exact
template an instruction list must follow, including per-commit `published` and
`signed` flags an agent should not recompute itself.

With a valid instruction list, an unpublished range reports
`execution.status: "executable"`. A range containing commits reachable from a
local remote-tracking ref reports `execution.status: "preview_only"` with
`requires_confirmation_artifact: true`, because rewriting published history
needs the separate `super-git.confirmation.v0.1` artifact. Any hard block (for
example a detached HEAD, an in-progress operation, a merge commit in range, or
an instruction list that does not cover the range) returns
`execution.status: "blocked"` with structured, repairable reason codes.

Staged and unstaged changes are allowed with a `working_tree_dirty` warning.
Preview never touches refs, the index, the working tree, or config. The basic
tree-preserving preview reads only; `drop` and reorder previews additionally
run replay prediction, which — like `predict rebase` — may write unreferenced,
gc-collectable objects into the object database (each clean replay step wraps
its result tree in a synthetic commit). Only malformed or wrong-schema
instruction input fails with `{ ok: false, error }`; content problems become
blocked plans instead. `reference_commands` are documentation only.

### Dropping commits

A `drop` op marks a commit whose patch should be removed from the final
history. Drop does not delete commit objects or "delete history": the branch
ref moves to a newly built chain in which the kept commits are replayed and
the dropped ones are absent. The full flow:

```bash
super-git preview history-edit --base main                          # survey
# edit the instruction template: change one op to "drop"
super-git preview history-edit --base main --instructions edit.json # plan
# the plan carries confirmation.required_phrase — a human types it into a
# super-git.confirmation.v0.1 artifact acknowledging the plan's reason codes
super-git execute --plan plan.json --confirmation confirm.json
super-git undo --token result.json                                  # if needed
```

Drop-specific contract, visible in the plan itself:

- The preview replays the kept commits internally (the Stage 7 predictor). A
  predicted conflict returns `execution.status: "blocked"` with reason code
  `predicted_conflict` and per-file stage evidence; nothing is ever resolved
  automatically. A clean prediction embeds `prediction.final_tree`, the exact
  tree execute must land on — the prediction is bound by `plan_id`, so it
  cannot be forged.
- Drop is always confirmation-gated (`preview_only`), regardless of published
  state, with reason code `tree_changing_drop` and the deterministic phrase
  `drop <N> commit(s) from <branch_ref> at <tip_commit> for plan
  <short-plan-id>`. When the range is also published, the reason codes name
  both, but the phrase stays the drop phrase.
- `drop` may be mixed with `pick` and `reword` only; mixing with
  `squash`/`fixup` blocks as `drop_with_fold_unsupported`. Dropping every
  commit in the range is allowed and moves the branch to `base` itself.
- Execute requires a clean working tree (untracked counts as dirty) and
  synchronizes the index and working tree to the new tip afterwards — the
  first history-edit family that touches the working tree. The plan states
  this as the non-volatile precondition
  `working_tree_clean_required_at_execute`.

### Reordering commits

Reorder changes commit order by permuting the `items` array. There is no
separate `op: "reorder"` and no position field; the list order is the planned
history order. Example: to swap the first two commits while keeping all three,
submit the same three `pick` items in the new order:

```json
{
  "schema_version": "super-git.instructions.v0.1",
  "action": "history_edit",
  "base": "main",
  "items": [
    { "commit": "<second-oldest>", "op": "pick" },
    { "commit": "<oldest>", "op": "pick" },
    { "commit": "<newest>", "op": "pick" }
  ]
}
```

Reorder-specific contract:

- Preview replays the commits internally and embeds a
  `prediction.kind: "reordered_commit_replay"` evidence block. A predicted
  conflict blocks with `predicted_conflict`; a clean replay is still blocked if
  it would change the final tree (`reorder_changes_final_tree`) or create an
  empty replay step in v0 (`reorder_creates_empty_commit`).
- Clean reorder is tree-preserving by contract: execute verifies
  `tree(new tip) == tree(old tip)`. It rebuilds commits from the first moved
  position using the prediction's per-step trees, moves only the branch ref,
  and leaves the working tree and index untouched. Dirty working trees are
  allowed with the same warning as other tree-preserving history edits.
- Reorder may mix with `pick` and `reword`. Mixing reorder with `drop` blocks
  as `reorder_with_drop_unsupported`; mixing with `squash`/`fixup` blocks as
  `reorder_with_fold_unsupported`.
- Unpublished reorder plans execute directly. Published reorder plans use the
  standard published-history confirmation phrase:
  `rewrite published history on <branch.ref> at <branch.tip_commit> for plan
  <short-plan-id>`.

## `predict merge --theirs <rev> [--ours <rev>]`

Predicts what a merge of two commits would do, without planning or running
anything. Contract:
`docs/internal/plans/2026-06-12-c9-0-conflict-prediction-contract.md`.

```bash
super-git predict merge --theirs feature            # ours defaults to HEAD
super-git predict merge --ours main --theirs feature
```

`predict` is a read verb, not a plan: the `super-git.conflict-prediction.v0.1`
result has no `plan_id`, nothing to execute, and nothing to undo. A predicted
conflict is a successful prediction, not an error:

```json
{ "prediction": { "status": "conflicted", "conflicted_files": [] } }
```

Per-file conflicts carry the index stages (1 = base, 2 = ours, 3 = theirs);
missing stages identify the conflict shape mechanically, such as a
modify/delete conflict having no stage 3. `notes[].kind` and `notes[].paths`
are stable across locales; `notes[].message` is localized free text for
display only.

Only inputs that cannot be predicted over fail with `{ ok: false, error }`:
`error.code` is `rev_not_found`, `no_merge_base`, `merge_tree_unsupported`
(Git older than 2.38), or `merge_tree_output_unrecognized`.

Prediction is commit-level and ignores the index and working tree, so a dirty
tree does not block it; the result always carries a `limitations` list,
including that a single merge prediction is not a rebase transcript. The
underlying `git merge-tree --write-tree` never touches refs, the index, or the
working tree, but may write unreferenced, gc-collectable objects into the
object database.

## `predict rebase --base <rev> --onto <rev>`

Predicts where replaying the linear `base..HEAD` range onto a new tip would
conflict, one step at a time. Same family and rules as `predict merge`: a
read verb with no `plan_id`, nothing to execute, nothing to undo, and a
predicted conflict is a successful prediction.

```bash
super-git predict rebase --base main --onto origin/main
```

The `super-git.rebase-prediction.v0.1` result carries one entry per replayed
commit in `steps` (oldest first), each embedding the same per-file conflict
shape as `predict merge` — the three-way roles rotate per step: the merge
base is the replayed commit's own parent, ours is the tip synthesized so
far, theirs is the replayed commit.

Prediction stops at the first conflicted step: composing further steps on
top of a conflicted tree would be meaningless, and the real resolution
changes every later step. `summary` makes the reach explicit:

```json
{
  "summary": {
    "status": "conflicted",
    "total_steps": 3,
    "predicted_steps": 2,
    "first_conflict_commit": "<oid>",
    "steps_not_predicted": ["<oid>"]
  }
}
```

When every step is clean, `summary.final_tree` is the predicted post-rebase
tree. `--onto` does not need any common ancestor with the range (the
per-step base is explicit, matching `git rebase --onto` semantics).

Structured errors (`{ ok: false, error }`): `rev_not_found`, `empty_range`
(nothing to replay), `merge_commit_in_range` and `root_commit_in_range`
(only linear single-parent ranges can be replayed), plus the shared
`merge_tree_unsupported` and `merge_tree_output_unrecognized`.

Each clean step wraps its result tree in an unreferenced, gc-collectable
synthetic commit, extending the `predict merge` object-database nuance to
commits; refs, the index, and the working tree are never touched.

## `execute --plan <file|-> [--confirmation <file|->]`

Executes a previously previewed plan after re-validation.

```bash
super-git execute --plan /tmp/super-git-plan.json > /tmp/super-git-result.json
super-git execute --plan - < /tmp/super-git-plan.json
super-git execute --plan /tmp/remove-plan.json --confirmation /tmp/remove-confirmation.json
```

Current support is intentionally limited to internal allowlisted actions:
`stage_changes`, executable `worktree_create` plans, confirmed
`worktree_remove` plans, and `history_edit` plans (executable, or `preview_only`
with a confirmation artifact). `execute` rejects stale plans, tampered plans,
unsupported actions, unsupported options, blocked worktree plans, and mismatched
repository state.

For `history_edit`, execute re-derives the plan from fresh state and requires an
identical plan id before writing, then rebuilds the commit chain with Git
plumbing (`commit-tree`), moves the branch ref by compare-and-swap, and
post-verifies the final tree. Leading unchanged picks keep their original
object ids; reorder additionally caps that prefix at the first moved position.
Author identity is preserved on every rewritten commit. For the tree-preserving
ops, including clean reorder, the final tree must be byte-identical to the
pre-execute tree and the working tree and index are never touched; successful
results carry a `restore_branch_tip_snapshot` undo token.

For plans containing `drop`, execute additionally requires a clean working
tree (untracked counts as dirty, surfacing as `working_tree_clean`), blocks
ignored files sitting on paths the new tip tracks
(`ignored_path_collision`), verifies the rebuilt tip against the plan's
predicted `final_tree` before the ref moves, and after the compare-and-swap
synchronizes the index and working tree to the new tip with
`git read-tree -u --reset`. A failure after the ref moved surfaces as
`execute_partial_failure` (the branch ref is already correct; the record
stays in `intent` state so undo and re-execute fail closed). Successful drop
results carry a `restore_branch_tip_and_worktree` undo token.

Unpublished ranges (`execution.status: "executable"`) execute directly and must
not carry a confirmation artifact. Published ranges and all drop plans
(`execution.status: "preview_only"`) require a separate
`super-git.confirmation.v0.1` artifact whose target, reason codes, undo
strategy, and CLI phrase match the plan — the published phrase is
`rewrite published history on <branch.ref> at <branch.tip_commit> for plan
<short-plan-id>`, the drop phrase is `drop <N> commit(s) from <branch.ref> at
<branch.tip_commit> for plan <short-plan-id>`; copy the exact
`confirmation.required_phrase` from the plan. The confirmation is authorization
only and never replaces fresh revalidation. The undo token still restores the
local branch tip but cannot un-publish anything already pushed.

Successful execute results currently use `schema_version` value
`"super-git.execute.v0.2"`. Undoable actions include an `undo_token`;
non-undoable destructive actions intentionally omit it.

`worktree_remove` is destructive and not automatically undoable. It requires a
separate `super-git.confirmation.v0.1` artifact, then re-scans the target
immediately before deletion. Execute removes only the linked worktree with
`git worktree remove <target>` without `--force`; it does not delete branch
refs, remote refs, commits, or history. Successful remove results intentionally
omit `undo_token`.

`--plan -` and `--confirmation -` cannot be used together because they cannot
both read independent JSON documents from the same stdin stream.

## `undo --token <file|->`

Undoes a supported write using the result from `execute`.

```bash
super-git undo --token /tmp/super-git-result.json
super-git undo --token - < /tmp/super-git-result.json
```

The token is treated as untrusted input. For `stage_changes`, `undo` validates
local registry provenance and index checksums before restoring the previous
index snapshot, and does not modify working-tree file contents. Other actions
have their own boundaries below; only `history_edit` `drop` undo deliberately
synchronizes the working tree (as the inverse of its execute).

For `worktree_create` results, `undo` validates the local execution record,
target worktree identity, lock/prunable state, HEAD/ref drift, and a clean
target working tree including ignored files before removing the linked
worktree. It uses `git worktree remove` without `--force`, does not delete
branch refs or history, and removes a parent directory created by `super-git`
only when that parent is empty.

For `history_edit` results, `undo` validates the local execution record's
provenance (its embedded token must match, so a forged or downgraded token
kind is refused), that the branch still points at the post-execute tip
(otherwise it refuses with `branch_advanced_since_execute`), that the
pre-execute tip commit still exists in the object store, and that no Git
operation is in progress. It then moves the branch ref back to the
pre-execute tip by compare-and-swap. For tree-preserving edits
(`restore_branch_tip_snapshot`) it never edits working-tree files or the
index. For drop results (`restore_branch_tip_and_worktree`) it is the
symmetric inverse of drop execute: before any write it requires a clean
working tree (untracked counts as dirty) and no ignored files on paths the
pre-execute tip tracks (`ignored_path_collision`), and after the ref restore
it synchronizes the index and working tree back to the pre-execute tip. A
sync failure after the ref was restored surfaces as `undo_partial_failure`
(the ref is correct; only the working-tree sync is unfinished).

A successful undo consumes the execution record, so the identical plan and
confirmation can be executed again — without that, the state-based plan id
would lock the same edit out of the branch forever. Either way the rewritten
commits simply become unreachable objects that normal Git maintenance
collects later; undo never deletes branch refs or history.

## `status [path]`

Shows detailed Git status for a repository path or the current directory.

```bash
super-git status
super-git status /path/to/repo
```

Use `inspect` for a high-level safety snapshot and `status` for detailed file
status.

## `wt list [path]`

Lists Git worktrees using Git porcelain output.

```bash
super-git wt list
super-git wt list /path/to/repo
```

Use `inspect.worktree_context` for the current worktree's family summary, and
`wt list` when the full list is needed.

## `repo save [path]` / `repo add <path>` / `repo list` / `repo forget`

Manages the local repository registry. The canonical command is `repo save`.
`repo add <path>` remains as a compatibility alias for `repo save <path>`.
Its JSON response also keeps the legacy `data.path` field for older automation.

```bash
super-git repo save
super-git repo save /path/to/repo
super-git repo add /path/to/repo
super-git repo list
super-git repo forget <id-or-name-or-path>
```

The registry is stored under the resolved app home. `SUPER_GIT_HOME` overrides
the OS-specific config location for tests, CI, dogfooding, and isolated agent
work. Writes always persist the v1 config shape.

Repository entries are worktree families, not individual linked worktrees.
Saving from the main worktree and saving from a linked worktree deduplicate by
Git common directory identity. The identity hash preserves path case so
case-sensitive filesystems can keep `/Repo/.git` and `/repo/.git` distinct.

```json
{
  "repository": {
    "id": "sha256:<git-common-dir-identity>",
    "name": "repo",
    "kind": "worktree_family",
    "main_worktree": "/path/to/repo",
    "git_common_dir": "/path/to/repo/.git",
    "saved_from": "/path/to/repo-feature"
  },
  "added": true
}
```

For bare-primary worktree families, `kind` is `bare_worktree_family` and
`main_worktree` is `null`.

`repo forget` removes a saved registry entry only. It never deletes repository
directories, linked worktrees, bare Git directories, `.git`, or working-tree
files.

Selectors:

- full repository `id`
- path-like selector such as `/path/to/repo`, `./repo`, a linked worktree path,
  a repository subdirectory, stored `saved_from`, stored `main_worktree`, or
  stored `git_common_dir`
- unique repository `name`

Plain words without path separators are treated as names. Use `./repo` or an
absolute path when you intend a filesystem path. If a selector matches multiple
saved repository families, even across selector kinds such as id and name, the
command fails and leaves the config unchanged.

Successful data includes explicit safety fields:

```json
{
  "target": "repo",
  "repository": {
    "id": "sha256:<git-common-dir-identity>",
    "name": "repo",
    "kind": "worktree_family",
    "main_worktree": "/path/to/repo",
    "git_common_dir": "/path/to/repo/.git",
    "saved_from": "/path/to/repo"
  },
  "removed": true,
  "matched_by": "name",
  "remaining_repositories": 0,
  "registry_only": true,
  "filesystem_deleted": false
}
```
