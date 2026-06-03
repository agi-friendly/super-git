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

## `repo add <path>` / `repo list`

Manages the local repository registry.

```bash
super-git repo add /path/to/repo
super-git repo list
```

The registry is stored in the OS-specific config path reported by `doctor`.
