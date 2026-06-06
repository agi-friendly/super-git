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

## `doctor`

Checks whether the local environment can run `super-git`.

```bash
super-git doctor
```

Reports the system Git version, OS, architecture, and config path.

## `config path` / `config show`

Reports the resolved `super-git` app home and config file.

```bash
super-git config path
super-git config show
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

Set `SUPER_GIT_HOME` to isolate tests, CI, dogfooding, or subagent work from the
real user config. Without it, `super-git` uses the OS-specific config location.

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

## `repo save [path]` / `repo add <path>` / `repo list`

Manages the local repository registry. The canonical command is `repo save`.
`repo add <path>` remains as a compatibility alias for `repo save <path>`.
Its JSON response also keeps the legacy `data.path` field for older automation.

```bash
super-git repo save
super-git repo save /path/to/repo
super-git repo add /path/to/repo
super-git repo list
```

The registry is stored under the resolved app home. `SUPER_GIT_HOME` overrides
the OS-specific config location for tests, CI, dogfooding, and isolated agent
work. Writes always persist the v1 config shape.

Repository entries are worktree families, not individual linked worktrees.
Saving from the main worktree and saving from a linked worktree deduplicate by
Git common directory identity.

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
