# Global Config And Saved Repositories Design

Status: implemented through C5-D; C5-E remains planned

This design inserts a small global configuration layer before worktree creation
preview. The goal is not to build a full profile system. The goal is to give
`super-git` a stable app home, a versioned config file, and a saved repository
registry that future worktree previews can use without hardcoded local
conventions.

## Why This Comes Before Worktree Create

Worktree creation preview needs to answer these questions before it can produce
a safe plan:

- Which repository family is the action for?
- Which base ref or branch is being used?
- Which parent directory should contain linked worktrees?
- What should the new worktree directory be named?
- How should branch names be converted into path-safe directory components?

Without global config, every preview would need verbose CLI flags or hardcoded
local conventions. Both choices are weak for an AI-first tool. A thin persistent
substrate lets the preview command derive defaults while still producing an
explicit plan.

## Terminology

- App home: the directory owned by `super-git` for global config and future app
  state.
- Config: user-controlled settings, such as worktree path templates.
- Repository registry: tool-managed saved repository families.
- Profile: multiple named config sets. This is not part of this stage.

The stage name is:

```text
Global Config And Saved Repositories
```

The implementation-facing pieces are:

```text
AppHome + ConfigStore + RepositoryRegistry
```

## App Home Resolution

`super-git` should resolve its app home in this order:

1. If `SUPER_GIT_HOME` is set, use that directory.
2. Otherwise, use the OS-specific config directory from
   `directories::ProjectDirs`.

`SUPER_GIT_HOME` is intentionally broader than `SUPER_GIT_CONFIG_DIR`. Future
state, cache, or registry files may live under the same root. Tests, CI,
dogfooding, and subagents must use `SUPER_GIT_HOME` so they do not mutate the
real user config.

The current project already uses `directories::ProjectDirs` and reports the
resolved config path from `doctor`. C5 should keep that behavior and add a
self-describing config path command.

C5 should keep the current `ProjectDirs` application identity unless there is an
explicit migration decision. Once global config becomes part of regular use,
changing the default app home becomes a migration problem instead of a naming
cleanup.

Planned output shape:

```json
{
  "ok": true,
  "data": {
    "home": "/tmp/super-git-test-home",
    "source": "env:SUPER_GIT_HOME",
    "config_file": "/tmp/super-git-test-home/config.json"
  }
}
```

When the OS-specific location is used, `source` should be `project_dirs`.

## Config File V1

C5 should keep a single `config.json` file for v1. JSON matches the existing
`ConfigStore`, avoids adding a TOML dependency, and is natural for AI agents.

The physical file can stay unified, but the code model should keep settings and
repositories conceptually separate:

```text
ConfigFile
  schema_version
  settings
  repositories
```

Initial shape:

```json
{
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
```

## Worktree Template Syntax

Templates must not use shell-style `$NAME` variables. They are easy to expand
accidentally in shells and are visually ambiguous in CLI examples.

Use brace-delimited variables instead:

```text
{main_path}.worktrees
{repo_name}__{ref_slug}
```

Supported variables for C5:

- `{main_path}`: absolute path to the main worktree when one exists.
- `{repo_name}`: display name derived from the saved repository family.
- `{ref_slug}`: path-safe representation of the selected ref or branch.

`ref_slug_algorithm = "path_safe_v1"` defines the intended slug contract. The
actual worktree preview implementation should enforce it before creating a
plan.

C5-D validates template structure before saving:

- Variables must use brace syntax such as `{ref_slug}`.
- Shell-style `$REF` and `${REF}` syntax is rejected.
- `parent_template` must include `{main_path}` exactly once and must not include
  `{ref_slug}` or a literal `..` path component.
- `name_template` must include `{ref_slug}` exactly once and must not include
  `{main_path}` or path separators.
- `ref_slug_algorithm` currently accepts only `path_safe_v1`.
- Invalid `config set-worktree-template` updates fail without rewriting the
  existing config file.

`path_safe_v1` should account for:

- slash and backslash separators
- invalid path characters
- Windows reserved device names such as `CON`, `PRN`, `AUX`, `NUL`, `COM1`, and
  `LPT1`
- trailing dots and spaces on Windows
- empty results
- case-insensitive collisions on macOS and Windows

## Saved Repository Registry

Implemented in C5-C. The registry saves worktree families, not individual
linked worktrees. Running `repo save` from a linked worktree saves the family
represented by the Git common directory.

Repository entry:

```json
{
  "id": "sha256:<git_common_dir_identity_hash>",
  "name": "naon-dnl",
  "kind": "worktree_family",
  "main_worktree": "/Users/hokkk/work/naon-dnl",
  "git_common_dir": "/Users/hokkk/work/naon-dnl/.git",
  "saved_from": "/Users/hokkk/work/naon-dnl.worktrees/naon-dnl__works-eml-base"
}
```

Identity should be based on the Git common directory reported by Git, not on a
raw display path. The existing worktree state code already relies on Git to
separate current git dir from common dir; C5 should reuse that principle.

Bare-primary families may have no main worktree:

```json
{
  "id": "sha256:<git_common_dir_identity_hash>",
  "name": "bare-project",
  "kind": "bare_worktree_family",
  "main_worktree": null,
  "git_common_dir": "/path/to/bare.git",
  "saved_from": "/path/to/linked-worktree"
}
```

Worktree creation previews may reject default templates for bare-primary
families until an explicit target parent path is supported.

Timestamps such as `created_at` and `last_seen_at` are intentionally not
required in v1. They are useful for dashboards, but they can add avoidable
formatting and write-on-read questions. They can be added later when repository
profile or dashboard work needs them.

## CLI Surface

Planned commands:

```text
super-git config path
super-git config show
super-git config validate

super-git repo save [path]
super-git repo list
super-git repo forget <id-or-name-or-path>

super-git config set-worktree-template \
  --parent-template '{main_path}.worktrees' \
  --name-template '{repo_name}__{ref_slug}'
```

Notes:

- `config path`, `config show`, `config validate`, `repo save`, `repo list`, and
  `config set-worktree-template` are implemented.
- Existing `repo add` should remain as a compatibility alias or wrapper.
- `repo forget` should remove an entry from the registry only. It must not
  delete repository or worktree files.
- `repo forget` is still planned.

## Safety Rules

C5 must not add shell hooks, post-create commands, pre-remove commands, or copy
patterns.

Path templates are data. Shell commands are executable behavior. They need a
separate preview/execute/confirmation model and should not be included in the
global config foundation.

Copy patterns are also deferred. They can accidentally copy secrets, large
directories, generated files, or OS-specific glob behavior into new worktrees.

## Preview Boundary

Config is preview input, not execute authority.

For future worktree creation:

1. `preview worktree-create` may read global config.
2. It must render and freeze the target path in the plan.
3. `execute --plan` must validate the resolved path from the plan.
4. `execute --plan` must not re-read global config and recalculate the target
   path.

This follows the same trust boundary as `reference_commands`: documentation or
config may explain how a plan was formed, but execute must rebuild trusted
actions from internal logic and validated plan data.

## Migration Rules

Config loading should be strict:

- Missing `schema_version` means v0.
- v0 is the current `{ "repositories": [...] }` style config and should migrate
  into v1 in memory.
- Legacy repository paths that no longer resolve to Git repositories are skipped
  during migration because they cannot be assigned a worktree-family identity.
- `schema_version == 1` is current.
- Unknown future versions must fail with an `unsupported_config_schema` style
  error instead of being partially interpreted.

Config saving should always write the current schema version using the existing
atomic write pattern.

## Implementation Slices

Proposed commit-sized slices:

```text
C5-0 docs(config): define app home, global config, and repository registry
C5-A feat(config): add app home resolver and config path/show
C5-B feat(config): add config schema v1 and migration
C5-C feat(repo): save and list registry-backed worktree families
C5-D feat(config): set and validate worktree templates
C5-E feat(repo): forget saved repositories
```

C5-0 through C5-D are implemented. C5-E may move later if the first
implementation needs a smaller scope.

## Out Of Scope

- Full profile system
- Multiple active profiles
- Shell hooks
- Copy patterns
- Dashboard statistics
- GUI
- Cross-process config locks
- Worktree creation execute/undo
