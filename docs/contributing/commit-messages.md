# Commit Messages

`super-git` is a Git safety tool, so its own Git history should be useful to
future maintainers and future agents.

A commit message does not need to repeat every changed line. The diff already
shows what changed. The message should explain why the change exists, which
judgments were made, what was verified, and what was intentionally deferred.

## Goals

Good commit messages help a reader answer:

- What is the smallest coherent slice this commit completed?
- Why was this direction chosen?
- What safety or compatibility risks were considered?
- What was verified before the commit was made?
- Did the author knowingly defer anything?

This matters most when someone reaches the commit through `git blame` or a
future debugging session. The message should preserve the author's reasoning at
the time, including uncertainty when that uncertainty is relevant.

## Subject Line

Use a concise conventional subject:

```text
<type>(<scope>): <summary>
```

Examples:

```text
feat(config): add app home resolver
fix(undo): require local registry provenance
docs: refresh public project documentation
```

Keep the subject focused on the product change. If the work belongs to a
planning slice such as `C5-A`, put that in the body as metadata instead of
crowding the subject.

## Body Depth

Choose the smallest useful message depth.

### One-line commits

Use only the subject when the change is self-explanatory:

- typo fixes
- simple wording updates
- mechanical renames with no behavioral judgment

### Short body commits

Use a short body for most feature, fix, and docs slices:

```text
feat(config): add app home resolver

Slice: C5-A

Why:
- Worktree features need a global app home before storing saved repository
  families or user-level settings.
- SUPER_GIT_HOME lets tests and agent sessions avoid the real user config.

Verification:
- cargo fmt --all --check
- cargo clippy --all-targets -- -D warnings
- cargo test
- git diff --check
```

### Full decision commits

Use a fuller body when the commit changes a safety contract, migration path, or
risky Git behavior:

```text
fix(undo): require local registry provenance

Slice: C4-D

Why:
- Undo tokens should not be trusted by themselves.
- The local registry proves that this checkout actually executed the matching
  plan before undo is allowed to change the index.

Notes:
- Undo still restores the Git index only; it does not edit working-tree file
  contents.
- Registry cleanup policy is intentionally deferred.

Verification:
- cargo fmt --all --check
- cargo clippy --all-targets -- -D warnings
- cargo test
- preview -> execute -> undo dogfood on a real repository
```

## Useful Sections

Use these sections when they add real value:

- `Slice:` for roadmap or plan identifiers such as `C5-A`.
- `Why:` for product intent or root cause.
- `Notes:` for important decisions made during implementation.
- `Deferred:` for known gaps intentionally left for a later slice.
- `Risk:` for compatibility, safety, migration, or data-loss considerations.
- `Verification:` for commands or dogfood evidence.

Do not add empty sections. A short accurate message is better than a long
template filled with noise.

## Deferred Work

If a known issue is intentionally not handled in the current slice, say so in
the commit body.

Examples:

```text
Deferred:
- Config schema versioning is left for C5-B.
- Worktree path templates are not added until the saved repository model exists.
```

This keeps future readers from mistaking a deliberate slice boundary for an
oversight.

## Verification

For normal code changes, use the same baseline as `AGENTS.md`:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
git diff --check
```

Docs-only commits should still run the Rust checks when practical. The project
is small, and keeping every slice green is part of the product culture.

## Anti-patterns

Avoid:

- listing every file changed instead of explaining intent
- vague subjects such as `update`, `fix`, or `changes`
- hiding known uncertainty that would help future debugging
- claiming verification that was not actually run
- mixing unrelated fixes into one message because they happened in the same
  session

The standard is not "write a long message." The standard is "leave enough
reasoning for the next maintainer to understand the decision."
