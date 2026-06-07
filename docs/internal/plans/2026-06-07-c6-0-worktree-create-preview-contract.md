# C6-0 Worktree Create Preview Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define the first safe worktree creation contract before any worktree creation write action is implemented.

**Architecture:** `worktree_create` extends the existing preview/execute/undo lifecycle instead of becoming a raw `git worktree` wrapper. Preview resolves a repository family, source ref, target path, family snapshot, and execute eligibility into a versioned plan. Execute must later rebuild trusted Git commands from typed plan data and fresh repository state, never from documentation commands.

**Tech Stack:** Rust workspace, serde JSON contracts, clap CLI, system `git`, `git worktree list --porcelain`, `git rev-parse`, SHA-256 plan/family fingerprints, integration tests with temporary Git repositories and linked worktrees.

---

## C6-0 Scope

C6-0 is a docs-only contract checkpoint for the next write family. It does not
add `preview worktree-create`, `execute`, or `undo` behavior.

The contract exists because `git worktree add` changes more than one visible
thing. It can create a filesystem directory, update Git worktree administration
metadata, occupy a branch, and leave partial state if an operation fails. The
safe product shape is therefore:

```text
inspect -> preview worktree-create -> execute --plan -> undo --token
```

## Non-negotiable Rules

The first worktree create implementation must follow these rules:

- Preview is read-only.
- No `--force` behavior.
- No `--guess-remote` behavior.
- No implicit local branch creation from remote-tracking branches.
- No `--target` override in the first implementation.
- No copy patterns.
- No shell hooks.
- A branch already checked out in another worktree is a hard block.
- `reference_commands` are documentation only.
- Config templates are preview input, not execute authority.
- Worktree create undo removes only a clean worktree created by `super-git`.
- Worktree create undo does not delete branch refs, remote refs, commits, or
  user-created files.

## Command Surface

Initial preview command:

```bash
super-git preview worktree-create --ref <branch-or-tag-or-commit>
super-git preview worktree-create --repo <id-or-name-or-path> --ref <branch-or-tag-or-commit>
```

`--repo` may be omitted only when the current working directory is inside a Git
worktree and the repository family can be inferred unambiguously.

When `--repo` is omitted outside a Git worktree, or when the family cannot be
identified, preview fails with a structured error such as:

```json
{
  "ok": false,
  "error": {
    "message": "could not preview worktree_create",
    "causes": [
      "repo_selector_required: No repository family could be inferred from the current directory. Pass --repo explicitly."
    ]
  }
}
```

Supported `--repo` selectors:

- saved repository id
- unique saved repository name
- path inside a Git worktree family

Ambiguous selectors fail before any plan is produced.

## Source Ref Policy

The first executable scope is deliberately narrow:

| Input kind | Preview | Execute in first write slice |
| --- | --- | --- |
| existing local branch | supported | supported when not checked out elsewhere |
| tag | supported | supported as detached HEAD |
| commit hash | supported | supported as detached HEAD |
| remote-tracking branch | recognized | blocked |
| unknown ref | blocked | blocked |
| ambiguous ref | blocked | blocked |

Remote-tracking branches are not silently converted into local branches. A
future slice may support an explicit shape such as:

```bash
super-git preview worktree-create --ref origin/foo --new-branch foo --track
```

That is out of scope for the first worktree create implementation.

## Target Path Policy

The first implementation resolves the target path only from the saved worktree
templates:

```json
{
  "parent_template": "{main_path}.worktrees",
  "name_template": "{repo_name}__{ref_slug}",
  "ref_slug_algorithm": "path_safe_v1"
}
```

`--target` and `--name` are excluded from the first implementation. This keeps
the first safety surface focused on deterministic template expansion and target
validation.

The target resolver must produce:

- parent path
- directory name
- final target path
- template variables used
- path safety checks
- parent creation policy

The default template may point at a missing parent directory such as
`{main_path}.worktrees` when a repository family has no linked worktrees yet.
The first implementation may allow execute to create that parent directory only
when all of these are true:

- the target parent path was resolved by preview from the saved config template
- the target parent's own parent already exists and is a directory
- the target parent does not already exist as a file or symlink
- the target parent is outside `.git` and outside every existing worktree
- the plan records `target.parent_creation.will_create = true`

If execute creates the parent, undo may remove that parent only when it is empty
after the created worktree is removed and only when the execution record proves
that `super-git` created it. Otherwise undo leaves the parent directory in
place.

The resolver must not create directories during preview.

## Ref Slug Policy

`path_safe_v1` converts a Git ref input into a stable path component. It must be
portable across macOS, Linux, and Windows.

Required cases:

- Replace each maximal run of slash separators, backslash separators, Windows
  invalid path characters (`<`, `>`, `:`, `"`, `|`, `?`, `*`), or ASCII control
  characters with one `-`.
- Trim trailing dots and spaces.
- Reject an empty result.
- Prefix Windows reserved device names such as `CON`, `PRN`, `AUX`, `NUL`,
  `COM1` through `COM9`, and `LPT1` through `LPT9` with `ref-`. The reserved
  name check is case-insensitive and applies to the basename before the first
  dot, so `CON.txt` becomes `ref-CON.txt`.
- Detect case-insensitive collisions against existing worktree names and target
  directory entries.

The slug algorithm is part of the plan contract. If the algorithm changes, the
config value and preview schema must change together.

## Family Snapshot

An external dashboard command is not required before worktree create preview.
However, the preview implementation needs an internal worktree-family snapshot.

The snapshot should be based on Git porcelain output, not human prose:

```bash
git worktree list --porcelain
```

Minimum snapshot fields:

```json
{
  "worktrees": [
    {
      "path": "/abs/main-or-linked-worktree",
      "kind": "linked",
      "head": "abc123",
      "branch": "refs/heads/main",
      "detached": false,
      "locked": false,
      "prunable": false
    }
  ],
  "branch_occupancy": [
    {
      "branch": "refs/heads/works/eml-base",
      "worktree_path": "/abs/existing-linked-worktree"
    }
  ]
}
```

The snapshot is an internal read model first. A public dashboard can reuse it
after the create preview contract proves useful.

## Execution Status In Preview

Risk metadata alone is not enough. The plan must say whether it can be executed
by `super-git execute`.

Recommended values:

```text
preview_only
executable
blocked
needs_human_review
```

Initial worktree create preview normally returns either `executable` or
`blocked`. `needs_human_review` is reserved for future explicit force-like
actions.

Blocked plans should still be useful. They should include enough structured
data for an agent or human to understand what was requested and why execution is
not available.

## Plan Schema

Do not create a worktree-specific schema namespace. Worktree creation should
extend the existing plan family:

```text
super-git.plan.v0.2
```

The reason for `v0.2` is that `super-git.plan.v0.1` is shaped around
`stage_changes` pathsets. Worktree creation needs typed fields for source refs,
resolved target paths, family snapshots, and execution status. Forcing those
fields into the v0.1 shape would make the contract harder to maintain.

Preview output remains wrapped by the global JSON envelope:

```json
{
  "ok": true,
  "data": {
    "schema_version": "super-git.plan.v0.2",
    "plan_id": "sha256:<canonical-plan-hash>",
    "action": {
      "kind": "worktree_create",
      "options": {
        "repo_selector": "naon-dnl",
        "ref": "works/eml-base"
      }
    },
    "repository": {
      "family_id": "sha256:<git-common-dir-identity>",
      "git_common_dir": "/abs/repo/.git",
      "main_worktree": "/abs/repo",
      "selected_from": "/abs/repo"
    },
    "config_used": {
      "source": "global_config",
      "config_home_source": "env:SUPER_GIT_HOME",
      "config_fingerprint": "sha256:<config-contract-hash>",
      "worktree_template": {
        "parent_template": "{main_path}.worktrees",
        "name_template": "{repo_name}__{ref_slug}",
        "ref_slug_algorithm": "path_safe_v1"
      }
    },
    "source_ref": {
      "input": "works/eml-base",
      "kind": "local_branch",
      "full_ref": "refs/heads/works/eml-base",
      "resolved_commit": "abc123",
      "supported_for_execute": true
    },
    "ref_policy": {
      "mode": "existing_local_branch",
      "will_create_branch": false,
      "will_detach_head": false,
      "will_track_upstream": false
    },
    "target": {
      "path": "/abs/repo.worktrees/repo__works-eml-base",
      "parent": "/abs/repo.worktrees",
      "name": "repo__works-eml-base",
      "exists": false,
      "parent_exists": true,
      "parent_is_directory": true,
      "parent_is_symlink": false,
      "parent_creation": {
        "allowed": true,
        "will_create": false,
        "removable_by_undo_if_empty": false
      },
      "inside_git_dir": false,
      "inside_existing_worktree": false,
      "case_insensitive_collision": false,
      "reserved_name_collision": false
    },
    "family_snapshot": {
      "fingerprint": "sha256:<family-state-hash>",
      "worktrees": [],
      "branch_occupancy": []
    },
    "preconditions": [
      { "code": "repo_family_resolved", "status": "passed" },
      { "code": "source_ref_supported", "status": "passed" },
      { "code": "branch_not_checked_out_elsewhere", "status": "passed" },
      { "code": "target_path_available", "status": "passed" }
    ],
    "execution": {
      "status": "executable",
      "super_git_execute_required": true,
      "raw_git_allowed": false,
      "suggested_super_git_command": ["super-git", "execute", "--plan", "<plan-file>"],
      "blocked_reasons": []
    },
    "risk": {
      "severity": "medium",
      "reversibility": "reversible_if_unchanged",
      "requires_human_confirmation": false
    },
    "effects": [
      "Create one linked worktree at the resolved target path.",
      "Check out the selected local branch in the new worktree."
    ],
    "reference_commands": {
      "semantics": "documentation_only",
      "never_execute_directly": true,
      "commands": [
        ["git", "worktree", "add", "/abs/repo.worktrees/repo__works-eml-base", "works/eml-base"]
      ]
    },
    "undo_strategy": {
      "kind": "remove_created_worktree_if_clean",
      "deletes_branch": false,
      "deletes_history": false
    },
    "undo_preview": {
      "kind": "remove_created_worktree_if_clean",
      "available_after_execute": true,
      "limitations": [
        "Undo removes only the worktree created by super-git.",
        "Undo refuses if the created worktree is dirty, locked, moved, or no longer matches the execute record.",
        "Undo does not delete branch refs or commits."
      ]
    }
  }
}
```

## Blocked Plan Example

If a local branch is already checked out in another worktree, preview should
return a structured blocked plan instead of asking Git to fail later.

```json
{
  "execution": {
    "status": "blocked",
    "super_git_execute_required": true,
    "raw_git_allowed": false,
    "suggested_super_git_command": null,
    "blocked_reasons": [
      {
        "code": "branch_already_checked_out",
        "severity": "hard_block",
        "details": {
          "branch": "refs/heads/works/eml-base",
          "worktree_path": "/abs/repo.worktrees/repo__works-eml-base"
        }
      }
    ]
  }
}
```

If a remote-tracking branch is selected, preview should recognize it but block
execute:

```json
{
  "source_ref": {
    "input": "origin/foo",
    "kind": "remote_tracking_branch",
    "full_ref": "refs/remotes/origin/foo",
    "resolved_commit": "abc123",
    "supported_for_execute": false
  },
  "execution": {
    "status": "blocked",
    "blocked_reasons": [
      {
        "code": "remote_tracking_branch_requires_local_branch_policy",
        "severity": "hard_block"
      }
    ]
  }
}
```

## Plan Hash Inputs

`plan_id` must bind the data that makes execution safe:

- schema version
- action kind and options
- repository family identity
- selected source ref and resolved commit
- ref policy
- resolved target path and safety results
- config fingerprint and template values used to resolve the path
- family snapshot fingerprint
- preconditions
- execution status
- blocked reason codes and details
- risk
- undo strategy

`plan_id` must exclude advisory prose:

- effects
- human-readable messages
- reference commands
- suggested `super-git` command
- timestamps

The v0.2 plan hash reuses the C4 canonical JSON rule: UTF-8 JSON object, sorted
object keys, no insignificant whitespace, and the same representation in tests
and production code. The v0.2 domain separator is:

```text
super-git-plan-v0.2\n
```

## Execute Contract For C6-C

`execute --plan` for `worktree_create` must:

1. Parse and validate the plan schema.
2. Recompute the plan hash.
3. Confirm action kind is allowlisted.
4. Confirm `execution.status` is `executable`.
5. Confirm repository family identity still matches.
6. Confirm source ref still resolves to the previewed commit.
7. Confirm branch occupancy still matches.
8. Confirm target path safety still matches.
9. Confirm no force-like or remote-tracking behavior is implied.
10. Write an execution-intent registry record before calling Git.
11. Rebuild the trusted `git worktree add` arguments from typed fields.
12. Run Git without shell interpretation.
13. Post-verify that the new worktree exists at the target path.
14. Post-verify that the new worktree HEAD/ref matches the plan.
15. Write a local execution record containing the worktree undo token.

If Git fails after creating partial state, execute must return a structured
failure with cleanup hints. It must not pretend the action simply failed with no
state change.

The execution-intent record is required because `git worktree add` can partially
write filesystem or Git metadata before returning failure. If `execute` cannot
write the intent record, it must fail before calling Git. Once Git may have
written state, every failure result must include durable machine-readable
recovery data:

```json
{
  "status": "failed_partial",
  "execution_record_path": "/abs/repo/.git/super-git/executions/<id>.json",
  "observed": {
    "target_path_exists": true,
    "worktree_list_entry_present": true
  },
  "cleanup": {
    "automatic_undo_available": false,
    "safe_next": "inspect_cleanup_record",
    "reason": "Post-verification failed after Git may have created worktree state."
  }
}
```

A partial failure record is not permission to delete anything blindly. It is a
provenance anchor for a future cleanup or undo command to re-inspect the target
state before acting.

## Undo Contract For C6-D

Worktree create undo means:

```text
remove the worktree directory and Git worktree admin entry created by super-git
```

It does not mean:

```text
delete branch refs, delete remote refs, delete commits, or reset history
```

Undo must validate:

- token schema
- local execution-record provenance
- target path
- target path is still a linked worktree
- target is not the main worktree
- target is not locked
- target HEAD/ref still matches the execute record
- target has no staged changes
- target has no unstaged changes
- target has no untracked files
- target has no ignored files
- target is still part of the expected family
- any parent directory marked as created by `super-git` is empty before undo
  removes it

After removal, undo must verify:

- target path no longer exists, or no longer contains the removed worktree
- `git worktree list --porcelain` no longer reports that target as an active
  worktree
- branch refs were not deleted by `super-git`
- a parent directory created by `super-git` was removed only if empty, otherwise
  it was left in place and reported

## Implementation Slices After C6-0

### C6-A: Ref slug and target path resolver

**Files:**
- Modify or create: `crates/super-git-core/src/config/*`
- Create: `crates/super-git-core/src/git/worktree_plan.rs`
- Modify: `crates/super-git-core/src/git/mod.rs`
- Test: core unit tests for slugging, template rendering, and path safety

Acceptance:

- `path_safe_v1` handles separators, invalid path characters, reserved device
  names, trailing dots/spaces, empty slugs, and case-insensitive collisions.
- Template rendering returns parent, name, final target path, and safety results.
- Missing target parent directories are either blocked or marked for
  `super-git` creation according to the parent creation policy.
- Resolver performs no filesystem writes.
- Bare-primary families without `main_worktree` fail for default templates.

### C6-B: `preview worktree-create`

**Files:**
- Create or modify: `crates/super-git-core/src/git/preview_worktree.rs`
- Modify: `crates/super-git-core/src/model.rs`
- Modify: `crates/super-git-cli/src/args.rs`
- Modify: `crates/super-git-cli/src/main.rs`
- Modify: `crates/super-git-cli/src/output.rs`
- Test: `crates/super-git-cli/tests/preview_worktree.rs`

Acceptance:

- `preview worktree-create --ref <ref>` works when the current directory
  identifies a worktree family.
- `preview worktree-create --repo <selector> --ref <ref>` works for saved
  repository id, unique name, and path selectors.
- Preview emits `{ ok: true, data: { schema_version, plan_id, action,
  repository, config_used, source_ref, ref_policy, target, family_snapshot,
  preconditions, execution, risk, effects, reference_commands, undo_strategy,
  undo_preview } }`.
- Existing local branch, tag, and commit inputs are covered.
- Remote-tracking branch input is recognized and blocked.
- Branch already checked out in another worktree is blocked.
- Target collision is blocked.
- Preview performs no writes.

### C6-C: Execute validated worktree create plans

Acceptance:

- `execute --plan` rejects unsupported plan schemas and unsupported actions.
- It accepts `worktree_create` only when the plan is executable.
- It revalidates source ref, target path, branch occupancy, family fingerprint,
  and repository identity immediately before writing.
- It never uses `reference_commands` as executable input.
- It creates one linked worktree and returns an undo token only after
  post-verification succeeds.
- Partial failures return structured cleanup information.

### C6-D: Undo unchanged worktree create

Implemented in C6-D.

Acceptance:

- Undo removes only a worktree created by `super-git` and recorded in the local
  execution record under the Git common directory.
- Undo refuses dirty, untracked, locked, moved, main, mismatched, or
  user-mutated worktrees.
- Undo does not delete branch refs or commits.
- Undo post-verifies that the worktree disappeared from `git worktree list
  --porcelain`.

## Deferred Features

These are product ideas, not part of the first worktree create contract:

- external worktree dashboard
- `--target` override
- `--name` override
- implicit remote branch tracking
- new local branch creation from remote-tracking branches
- force branch reuse
- copy patterns
- pre-create or post-create shell hooks
- opening editors or terminals after creation
- `task-start` orchestration commands

## C6-0 Self-review Checklist

- [x] The contract does not allow preview to write.
- [x] The first command surface has no `--force`, `--guess-remote`, `--target`,
  copy patterns, or hooks.
- [x] Remote-tracking branches are recognized but blocked.
- [x] Branch occupancy produces a hard block.
- [x] Target path policy is deterministic and config-derived.
- [x] Target parent creation policy is explicit.
- [x] `path_safe_v1` replacement and reserved-name behavior is deterministic.
- [x] Plan hash canonicalization and domain separator are explicit.
- [x] Partial Git-write failures require durable recovery data.
- [x] `reference_commands` are explicitly documentation-only.
- [x] Undo scope excludes branch and history deletion.
- [x] Future implementation slices are small enough to review independently.
