# Public Docs Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the repository documentation into a public GitHub entrypoint while preserving early ideation notes as archive material.

**Architecture:** The root `README.md` becomes the human-facing product overview. `docs/README.md` becomes the documentation router. Early `research/` and `dual-brain/` notes move under `docs/archive/original-notes/` so they remain available without defining the current product contract.

**Tech Stack:** Markdown documentation, Rust workspace verification commands, system `git`, existing CLI contracts.

---

## Tasks

### Task 1: Archive early notes without deleting history

**Files:**
- Move: `research/` -> `docs/archive/original-notes/research/`
- Move: `dual-brain/` -> `docs/archive/original-notes/dual-brain/`
- Create: `docs/archive/original-notes/README.md`

- [x] Move old ideation folders with `git mv`.
- [x] Add a short archive README explaining that these files are historical inputs, not the current source of truth.

### Task 2: Promote the current public documentation surface

**Files:**
- Rewrite: `README.md`
- Create: `docs/README.md`
- Rename: `docs/setup.md` -> `docs/getting-started.md`

- [x] Make the root README answer what the project is, why it exists, what works now, and where to read next.
- [x] Make `docs/README.md` a router for humans and agents.
- [x] Keep setup instructions but rename them to a public-friendly getting-started guide.

### Task 3: Document command and safety contracts

**Files:**
- Create: `docs/command-reference.md`
- Create: `docs/safety-model.md`
- Modify: `docs/architecture.md`
- Modify: `docs/roadmap.md`

- [x] Describe JSON envelope, `--human`, and command contracts in one place.
- [x] Describe the `inspect -> preview -> execute -> undo` lifecycle and trust boundaries.
- [x] Keep architecture and roadmap aligned with the implemented CLI.

### Task 4: Add AI contributor guide

**Files:**
- Create: `AGENTS.md`

- [x] Tell new AI sessions what to read first.
- [x] State safety rules: JSON-first, preview before execute, no raw Git mutation from hints.
- [x] Include verification commands and commit expectations.

### Task 5: Verify and commit

**Commands:**

```bash
rg -n "docs/setup|docs/superpowers|research/|dual-brain/" README.md AGENTS.md docs --glob '!docs/archive/**' --glob '!docs/internal/**'
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git diff --check
```

- [x] Fix stale links if `rg` finds any old public paths.
- [x] Run the verification commands.
- [x] Commit the docs-only change.
