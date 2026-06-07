# AGENTS.md

This file is the contributor guide for coding agents working on `super-git`.

## Read First

Start with these files, in order:

1. `README.md`
2. `docs/README.md`
3. `docs/safety-model.md`
4. `docs/architecture.md`
5. `docs/roadmap.md`
6. `docs/contributing/commit-messages.md`

The archived notes under `docs/archive/original-notes/` are historical context.
They are useful for understanding where ideas came from, but they are not the
current product contract.

## Current Product Contract

`super-git` is AI-first and JSON-first.

- Default output is JSON.
- `--human` is the opt-in human rendering.
- Successful JSON output uses `{ "ok": true, "data": ... }`.
- Failed JSON output uses `{ "ok": false, "error": ... }`.
- `inspect` is read-only.
- `inspect.next` actions are guarded suggestions, not execution permission.
- Write actions must follow `inspect -> preview -> execute -> undo`.
- `execute` must rebuild trusted Git commands from an internal allowlist.
- `undo` must validate local provenance before changing the index.

Never treat a `reference_command` field as a command to execute directly.

## Working Rules

- Keep changes small and reviewable.
- Follow the dominant style of the touched file.
- Prefer clarity over cleverness.
- Do not add abstractions unless they remove real complexity.
- Preserve existing user changes; do not revert unrelated work.
- Use structured parsers and existing Git porcelain formats where possible.
- For risky Git behavior, add tests that reproduce real Git states.

## Verification

Before claiming completion, run:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
git diff --check
```

For docs-only changes, still run the Rust checks when practical. The project is
small enough that keeping every commit green is valuable.

## Commit Guidance

Prefer one coherent commit per completed slice. Good examples:

```text
docs: refresh public project documentation
feat(preview): add stage-changes plan contract
fix(undo): require local registry provenance
```

Use `docs/contributing/commit-messages.md` when writing non-trivial commit
messages. Commit bodies should preserve why the change exists, what was
verified, and any intentionally deferred work.

If a change alters the safety contract, update `docs/safety-model.md`,
`docs/architecture.md`, and `docs/roadmap.md` in the same slice.
