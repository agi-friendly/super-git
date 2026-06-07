# super-git Roadmap

This roadmap keeps the project small, testable, and safety-first. Each stage
should be useful before the next stage starts.

## Current Position

The project has a working read-side inspection layer and the first write-side
safety loop:

```text
inspect -> preview stage-changes -> execute --plan -> undo --token
```

The next stages should expand this lifecycle carefully instead of adding raw Git
wrappers.

## Stage 1: CLI Skeleton And Inspect (implemented)

- Rust workspace
- `super-git-core` and `super-git-cli`
- `super-git` CLI binary
- JSON-first output with `--human`
- `doctor`
- `repo add`
- `repo save`
- `repo list`
- `status`
- `inspect`
- `wt list`
- repository config storage
- worktree/status/state parsing
- inspect guardrails, warnings, risk hint, and next-action candidates

## Stage 2: Preview/Execute/Undo Foundation (implemented for `stage_changes`)

- `preview stage-changes`
- plan hash and state fingerprint
- execute trust boundary
- precondition mismatch handling
- internal write allowlist
- undo token generation
- local undo registry provenance
- index-only undo for staged changes

## Stage 3: Global Config And Saved Repositories

Implemented:

- app home resolver with `SUPER_GIT_HOME` override
- OS-specific config path from `directories::ProjectDirs`
- `config.json` schema version 1
- v0 config migration to v1 in memory
- saved repository registry by worktree family
- registry-backed `repo list`
- worktree parent/name templates
- `{main_path}`, `{repo_name}`, and `{ref_slug}` template variables
- `config validate` for templates and saved repository registry shape
- `config set-worktree-template`
- `repo forget`, including registry-only safety and ambiguous selector checks
- JSON parse-error coverage and human smoke coverage for C5 config/repo commands

Next:

- no shell hooks, copy patterns, or profile system

## Stage 4: Safe Worktree Create Preview

Implemented so far:

- `path_safe_v1` ref slug rendering
- config-derived target path resolver
- target parent creation policy
- target path safety flags for existing paths, Git dir nesting, existing
  worktree nesting, case-insensitive name collisions, and reserved names
- contract checkpoint for `worktree_create` preview
- internal worktree-family snapshot based on Git porcelain data
- `preview worktree-create --ref <ref>`
- `preview worktree-create --repo <id-or-name-or-path> --ref <ref>`
- source ref classification for local branch, tag, commit, remote-tracking
  branch, and unknown ref
- remote-tracking branch input is recognized and blocked
- branch occupancy hard blocks when a branch is already checked out elsewhere
- explicit `execution.status` and structured blocked reasons
- unblocked plans report `preview_only` until C6-C implements execute support
- clear risk and reversibility metadata
- target path resolved from config and frozen in the plan
- no `--force`, `--guess-remote`, `--target`, copy patterns, or shell hooks in
  the first implementation
- `execute` rejects `super-git.plan.v0.2` until validated worktree execution is
  implemented

Next:

- ambiguous ref handling beyond exact local branch/tag/remote/commit lookup
- full `locked` and `prunable` worktree snapshot parsing
- execute validated worktree creation plans

## Stage 5: Worktree Create/Remove Execute

- execute validated worktree creation plans
- execute validated worktree removal plans
- protect dirty worktrees and untracked files
- require clear confirmation rules for destructive removal
- undo unchanged worktree creation without deleting branch refs or commits
- provide cleanup guidance where true undo is not possible

## Stage 6: Repository Profile And Dashboard

- lightweight repository profile for scale/history hints
- repository size, commit count, initial commit, last commit, and remotes where
  performance is acceptable
- clear distinction between cheap always-on fields and expensive opt-in fields
- multi-repository dashboard model
- registered repository summaries
- stable JSON for future UI surfaces

## Stage 7: Guides For Agents

- `super-git guide list`
- conflict/rebase/worktree/super-git usage guides
- documentation-oriented output for weaker or older coding models
- no fake execution magic: guides teach how to use the existing safety contract

## Stage 8: Desktop Prototype

- thin desktop UI over the core/CLI contracts
- repository list
- inspect summary
- status/worktree views
- preview/execute/undo flow visualization

## Stage 9+: Integrations And Advanced Git

- Windows Explorer integration
- macOS Finder integration
- Linux file-manager integration
- plugin or extension system
- conflict helper
- patch create/apply workflow
- interactive rebase helper
- reflog and branch browser
