# super-git

AI-first Git safety tooling for humans and coding agents.

`super-git` is a CLI-first experiment that makes Git's hidden state machine
explicit before any write action happens. The core product contract is:

```text
inspect -> preview -> execute -> undo
```

Undo is action-specific, not promised for every write. Destructive flows must
use explicit confirmation and recovery contracts when automatic undo cannot be
honestly proven.

The tool is designed for AI agents first, but the same properties make it useful
for humans: clear state, structured output, dry-run planning, guarded execution,
and undo provenance.

## Why This Exists

Git is powerful, but its critical state is spread across status output, internal
files, reflog, index state, worktree metadata, and command-specific edge cases.
Humans often rely on IDEs or GUI tools to keep that state visible. Coding agents
usually operate from a terminal and must reconstruct the state from scratch.

`super-git` aims to give both humans and agents a safer contract:

- See the repository state in one structured snapshot.
- Preview write actions before changing the repository.
- Execute only validated plans, not arbitrary command strings.
- Undo supported writes through local provenance checks.
- Keep JSON as the default output, with `--human` for terminal reading.

## Current Status

This project is still early, but the first safety loop exists.

Implemented today:

- `super-git inspect [path]`
  - repository root
  - worktree family context
  - HEAD and detached/unborn state
  - upstream ahead/behind
  - working-tree summary and conflicts
  - in-progress Git operation
  - warnings, risk hint, summary, and guarded next-action candidates
- `super-git preview stage-changes`
  - builds a read-only plan for staging current unstaged/untracked changes
- `super-git preview worktree-create --ref <ref> [--repo <selector>]`
  - builds a read-only plan for creating a linked worktree
  - recognizes blocked cases such as occupied branches, remote-tracking refs,
    and target collisions without writing
- `super-git preview worktree-remove --worktree <absolute-linked-worktree-path>`
  - builds a read-only destructive-action plan for removing an existing linked
    worktree
  - reports strict hard blocks, human confirmation requirements, no automatic
    undo, and manual recovery hints without deleting anything
- `super-git preview history-edit --base <ref> [--instructions <file|->]`
  - without `--instructions`, returns a read-only survey of the editable
    `base..HEAD` range (commits, published state, signatures, hard blocks)
  - with a `super-git.instructions.v0.1` document (`pick`/`reword`/`squash`/
    `fixup`/`drop` per commit, plus reorder by changing the item order),
    builds a rewrite plan; `pick`/`reword`/`squash`/`fixup` and clean reorder
    plans are tree-preserving, while `drop` removes a commit's patch from the
    final history
  - unpublished tree-preserving plans execute directly; published ranges and
    all `drop` plans produce a `preview_only` plan that requires a separate
    human confirmation artifact
  - `drop` and reorder plans embed replay predictions (a predicted conflict
    blocks the plan; nothing is ever auto-resolved); drop carries the predicted
    `final_tree` that execute must land on, while reorder is allowed only when
    the predicted final tree equals the old tip's tree
- `super-git predict merge --theirs <rev> [--ours <rev>]`
  - predicts merge conflicts between two commits via `git merge-tree`,
    reporting per-file conflicts by index stage; a predicted conflict is a
    successful prediction, not an error
  - a read verb, not a plan: no `plan_id`, nothing to execute or undo, and
    never any automatic conflict resolution
  - touches no refs, index, or working-tree state (it may write unreferenced,
    gc-collectable objects into the object database)
- `super-git predict rebase --base <rev> --onto <rev>`
  - predicts where replaying the linear `base..HEAD` range onto a new tip
    would conflict, step by step, stopping at the first predicted conflict
  - reports per-step conflicts in the same shape as `predict merge`, plus a
    summary with the predicted reach and (when fully clean) the final tree
  - same read-verb rules: no plan, no execute/undo, no automatic conflict
    resolution, no ref/index/working-tree writes
- `super-git execute --plan <file|-> [--confirmation <file|->]`
  - re-validates the plan and state before writing
  - executes only internal allowlisted actions: `stage_changes`, executable
    `worktree_create` plans, confirmed `worktree_remove` plans, and
    `history_edit` plans
  - writes local provenance before reporting success
  - for destructive `worktree_remove`, requires a separate confirmation
    artifact and returns no automatic undo token
  - for `history_edit`, rebuilds commits with `commit-tree` (author identity
    preserved), moves the branch ref with compare-and-swap, verifies the final
    tree against the expected one, and requires the confirmation artifact when
    the range is published
  - for `history_edit` reorder plans, uses the replay prediction to rebuild
    commits in the requested order while keeping the final tree identical; it
    is ref-only and does not require or clean the working tree
  - for `history_edit` plans containing `drop`, additionally requires a clean
    working tree (untracked counts as dirty), blocks ignored files sitting on
    paths the new tip tracks, verifies the rebuilt tip against the predicted
    `final_tree` before the ref moves, and synchronizes the index and working
    tree to the new tip afterwards — the typed phrase
    `drop <N> commit(s) from <branch_ref> at <tip> for plan <short-plan-id>`
    is always required
- `super-git undo --token <file|->`
  - treats token input as untrusted
  - for `stage_changes`, validates repository, snapshot checksums, current
    index checksum, and local registry provenance before restoring the
    pre-execute index
  - for `worktree_create`, validates local execution-record provenance, target
    worktree identity, clean state, lock state, and HEAD/ref drift before
    removing the linked worktree
  - for `history_edit`, validates local execution-record provenance and that
    the branch still points at the post-execute tip, then restores the
    pre-execute branch tip with compare-and-swap (local only; it cannot
    un-publish pushed history); a successful undo consumes the execution
    record so the same plan can be executed again
  - for `history_edit` drop results (`restore_branch_tip_and_worktree`),
    symmetrically requires a clean working tree and no ignored-path
    collisions against the pre-execute tip, then synchronizes the index and
    working tree back to it after the ref restore
  - never deletes branch refs or history; working-tree files change only in
    the drop family's documented synchronization, never by content editing
- Supporting commands: `doctor`, `config path`, `config show`,
  `config validate`, `config set-worktree-template`, `repo save`, `repo add`,
  `repo list`, `repo forget`, `status`, `wt list`

## Quick Start

```bash
cargo run -p super-git-cli -- doctor
cargo run -p super-git-cli -- inspect
```

In a disposable repository or throwaway worktree with unstaged/untracked
changes:

```bash
cargo run -p super-git-cli -- preview stage-changes > /tmp/super-git-plan.json
cargo run -p super-git-cli -- execute --plan /tmp/super-git-plan.json > /tmp/super-git-result.json
cargo run -p super-git-cli -- undo --token /tmp/super-git-result.json
```

By default, every command returns a JSON envelope:

```json
{ "ok": true, "data": {} }
```

Failures use the same contract:

```json
{ "ok": false, "error": { "message": "...", "causes": [] } }
```

Use `--human` when reading directly in a terminal:

```bash
cargo run -p super-git-cli -- --human inspect
```

## Documentation

- [Documentation map](docs/README.md)
- [Getting started](docs/getting-started.md)
- [Command reference](docs/command-reference.md)
- [Safety model](docs/safety-model.md)
- [Architecture](docs/architecture.md)
- [Roadmap](docs/roadmap.md)
- [ADR 0001: CLI First](docs/adr/0001-cli-first.md)
- [Commit messages](docs/contributing/commit-messages.md)
- [Archived original notes](docs/archive/original-notes/README.md)

## Development

Required tools:

- Git
- Rust toolchain
- Cargo
- rustfmt
- Clippy

Verification:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Non-goals For Early Versions

- Reimplementing Git
- Replacing mature Git GUI tools
- Running raw Git commands from `inspect` suggestions
- Building a desktop UI before the CLI/core contract is stable
- Building a plugin system before the safety lifecycle proves itself

Desktop and GUI ideas are still part of the long-term dream. For now, the
project is building the safe core that those interfaces can later wrap.
