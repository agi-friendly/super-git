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
local-branch policy exists. Occupied local branches and target path collisions
also return blocked plans instead of letting Git fail later.

`preview worktree-create` does not create directories, worktrees, config files,
or Git worktree metadata. Unblocked plans currently use
`execution.status: "preview_only"` because `execute` does not yet run
`worktree_create` plans.

## `execute --plan <file|->`

Executes a previously previewed plan after re-validation.

```bash
super-git execute --plan /tmp/super-git-plan.json > /tmp/super-git-result.json
super-git execute --plan - < /tmp/super-git-plan.json
```

Current support is intentionally limited to the internal `stage_changes`
allowlist. `execute` rejects stale plans, tampered plans, unsupported actions,
unsupported options, and mismatched repository state.

## `undo --token <file|->`

Undoes a supported write using the result from `execute`.

```bash
super-git undo --token /tmp/super-git-result.json
super-git undo --token - < /tmp/super-git-result.json
```

The token is treated as untrusted input. `undo` validates local registry
provenance and index checksums before restoring the previous index snapshot.
It does not modify working-tree file contents.

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
