# Safety Model

`super-git` treats Git as a powerful state machine that should be inspected and
validated before writes happen.

The current lifecycle is:

```text
inspect -> preview -> execute -> undo
```

## Principles

### JSON First

Automation should not scrape prose when structured data is available.

- JSON is the default.
- `--human` is opt-in.
- Success and failure both use predictable envelopes.

### Inspect Is Read-only

`inspect` answers:

- Where am I?
- Which worktree family am I in?
- What is HEAD?
- Is the repo ahead or behind upstream?
- Are there staged, unstaged, untracked, or conflicted paths?
- Is a Git operation in progress?
- What actions look safe enough to preview next?

`inspect` never grants execution permission.

### Preview Produces A Contract

`preview` creates a plan with action kind, preconditions, state fingerprint,
resolved paths, risk metadata, and undo strategy.

The plan is not a shell script. Documentation fields such as
`reference_commands` must not be executed directly.

### Execute Revalidates

`execute` must re-check the plan before writing:

- schema version
- plan hash
- supported action kind
- supported options
- repository root
- operation state
- fingerprint
- resolved pathset

Git commands are rebuilt from internal allowlists, not copied from the plan.

### Undo Requires Provenance

`undo` treats token input as untrusted.

For the current `stage_changes` action, undo is index-only:

- validate token schema
- validate repository identity
- validate snapshot path
- validate snapshot checksum
- validate local undo registry record created by `execute`
- validate current index checksum
- restore the pre-execute index snapshot only if the current index still matches
  the execute result

`undo` does not edit working-tree files.

## Current Write Boundary

Only one write action exists today:

```text
stage_changes
```

It stages the unstaged/untracked pathset captured by `preview`, but only after
`execute` confirms that the pathset and fingerprint still match.

Future actions must earn their way into the allowlist with tests and docs.

Global config and repository-registry writes are separate from Git repository
writes. For example, `repo forget` edits only `super-git`'s config registry and
must not delete repository directories, worktrees, bare Git directories, `.git`,
or working-tree files. Ambiguous `repo forget` selectors fail before writing,
including cross-kind matches such as one repository id and another repository
name. Because saved repositories become preview input for later worktree
actions, `config validate` treats malformed registry entries as invalid instead
of silently accepting arbitrary ids or relative paths.

Worktree creation is the next Git write family, but it is not a raw
`git worktree add` wrapper. The preview contract is documented in
`docs/internal/plans/2026-06-07-c6-0-worktree-create-preview-contract.md`.
`preview worktree-create` is read-only, does not use `--force` or
`--guess-remote`, does not imply remote branch tracking, and hard-blocks a
branch that is already checked out in another worktree. Target paths are
resolved from config during preview and frozen into `super-git.plan.v0.2`;
execute must not re-expand config templates as trusted authority. `execute`
still rejects `worktree_create` plans until the write-side validation slice is
implemented.

Worktree create undo is intentionally narrow: remove the clean linked worktree
created by `super-git` when local provenance and state checks still match. It
does not delete branch refs, remote refs, commits, or user-created files.

## Risk Vocabulary

The project is converging on a two-axis risk model:

- severity: how much damage a wrong action can cause
- reversibility: how confidently the action can be undone

The current implementation already separates read-only inspect data, preview
plans, guarded execute, and registry-backed undo. Future work will expand this
into richer warnings and human-confirmation rules for high-risk actions.
