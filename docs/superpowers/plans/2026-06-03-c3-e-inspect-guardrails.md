# C3-E Inspect Guardrails Implementation Plan

> **Historical note:** this file is the original C3-E implementation plan. Later
> commits may bump inspect schema versions or rename fields; use README and
> architecture/setup docs for the current public contract.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Evolve `super-git inspect` from a raw state snapshot into a versioned, lightly interpreted safety snapshot without entering write execution yet.

**Architecture:** Keep Git reads in `crates/super-git-core/src/git/state.rs`, keep serializable contracts in `crates/super-git-core/src/model.rs`, and keep CLI rendering in `crates/super-git-cli/src/output.rs`. C3-E adds only derived metadata and guardrails from the already-read state; it must not add network fetches or write to the repository.

**Tech Stack:** Rust workspace, serde JSON contracts, clap CLI integration tests, real temporary Git repositories in `crates/super-git-cli/tests/inspect.rs`.

---

## File Structure

- Modify `crates/super-git-core/src/model.rs`
  - Add inspect schema/version constants or fields.
  - Add upstream comparison basis.
  - Add warning, summary, current-state risk hint, and next guardrail data models.
  - Replace the final inspect action surface from `allowed_next` to `next` only when Task 3 lands.
- Modify `crates/super-git-core/src/git/state.rs`
  - Populate upstream comparison basis while reading upstream.
  - Compute warnings, summary, current-state risk hints, and next guardrails as pure functions from `RepoState` inputs.
  - Preserve the principle that `inspect` never mutates the repository and never contacts the network.
- Modify `crates/super-git-cli/src/output.rs`
  - Include new JSON fields under the existing `{ ok: true, data: ... }` envelope.
  - Keep human output compact; warnings and next guardrails should be visible only when non-empty or useful.
- Modify `crates/super-git-cli/tests/inspect.rs`
  - Update action helper from `allowed_next` to `next.allowed` in Task 3.
  - Add integration assertions for schema version, upstream comparison basis, warnings, summary/risk_hint, and guardrail buckets.
- Modify docs only when JSON contract changes are user-facing:
  - `README.md`
  - `docs/setup.md`
  - `docs/architecture.md`
  - `docs/roadmap.md`

---

### Task 1: Additive Schema Version + Upstream Basis + Warnings

**Files:**
- Modify: `crates/super-git-core/src/model.rs`
- Modify: `crates/super-git-core/src/git/state.rs`
- Modify: `crates/super-git-cli/src/output.rs`
- Modify: `crates/super-git-cli/tests/inspect.rs`

- [ ] **Step 1: Add failing integration assertions**

In `crates/super-git-cli/tests/inspect.rs`, add assertions to an existing clean repo test:

```rust
assert_eq!(json["data"]["schema_version"], "super-git.inspect.v0.1");
assert!(json["data"]["warnings"].as_array().expect("warnings array").is_empty());
```

In `inspect_reports_upstream_ahead`, add:

```rust
assert_eq!(upstream["comparison_basis"], "local_tracking_ref");
assert_eq!(upstream["comparison_status"], "ok");
let warnings = json["data"]["warnings"].as_array().expect("warnings array");
assert!(warnings.iter().any(|w| w["code"] == "upstream_freshness_unknown"));
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
cargo test -p super-git-cli --test inspect inspect_clean_repo_reports_branch_and_no_operation inspect_reports_upstream_ahead
```

Expected: FAIL because `schema_version`, `warnings`, and `upstream.comparison_basis` do not exist yet.

- [ ] **Step 3: Add serializable models**

In `crates/super-git-core/src/model.rs`, add:

```rust
pub const INSPECT_SCHEMA_VERSION: &str = "super-git.inspect.v0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamComparisonBasis {
    LocalTrackingRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamComparisonStatus {
    Ok,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectWarning {
    pub code: String,
    pub severity: WarningSeverity,
    pub message: String,
}
```

Update `UpstreamInfo`:

```rust
pub struct UpstreamInfo {
    pub name: String,
    pub ahead: u32,
    pub behind: u32,
    pub comparison_basis: UpstreamComparisonBasis,
    pub comparison_status: UpstreamComparisonStatus,
}
```

Update `RepoState`:

```rust
pub warnings: Vec<InspectWarning>,
```

- [ ] **Step 4: Populate warnings and upstream basis**

In `read_upstream`, populate:

```rust
comparison_basis: UpstreamComparisonBasis::LocalTrackingRef,
comparison_status,
```

Add a pure helper in `state.rs`:

```rust
fn compute_warnings(upstream: &Option<UpstreamInfo>) -> Vec<InspectWarning> {
    let mut warnings = Vec::new();
    if let Some(upstream) = upstream {
        warnings.push(InspectWarning {
            code: "upstream_freshness_unknown".to_string(),
            severity: WarningSeverity::Low,
            message: "Ahead/behind is based on the local tracking ref; fetch before treating remote state as current.".to_string(),
        });
        if upstream.comparison_status == UpstreamComparisonStatus::Failed {
            warnings.push(InspectWarning {
                code: "upstream_comparison_failed".to_string(),
                severity: WarningSeverity::Medium,
                message: "Upstream name was found, but ahead/behind comparison failed; counts should not be trusted.".to_string(),
            });
        }
    }
    warnings
}
```

Call it from `read_state` after `upstream` is read.

In `read_upstream`, do not silently treat `rev-list` failure or garbage as a trustworthy `0/0`. Keep `ahead` and `behind` as `0` for the current additive schema, but mark `comparison_status` as `Failed` whenever the command fails or the output does not contain two parseable integers.

- [ ] **Step 5: Render JSON and human warning output**

In `print_inspect` JSON, include:

```rust
"schema_version": super_git_core::model::INSPECT_SCHEMA_VERSION,
"warnings": state.warnings,
```

In human mode, after upstream output:

```rust
if !state.warnings.is_empty() {
    println!("Warnings:");
    for warning in &state.warnings {
        println!("  - {}: {}", warning.code, warning.message);
    }
}
```

- [ ] **Step 6: Run focused and full tests**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/super-git-core/src/model.rs crates/super-git-core/src/git/state.rs crates/super-git-cli/src/output.rs crates/super-git-cli/tests/inspect.rs
git commit -m "feat(inspect): version state output and expose upstream basis"
```

---

### Task 2: Summary + Inspect Risk Hint

**Files:**
- Modify: `crates/super-git-core/src/model.rs`
- Modify: `crates/super-git-core/src/git/state.rs`
- Modify: `crates/super-git-cli/src/output.rs`
- Modify: `crates/super-git-cli/tests/inspect.rs`

- [ ] **Step 1: Add failing assertions for summary and inspect risk**

In `inspect_clean_repo_reports_branch_and_no_operation`, assert:

```rust
assert_eq!(json["data"]["summary"]["state"], "ready");
assert!(json["data"]["summary"]["codes"]
    .as_array()
    .expect("summary codes")
    .iter()
    .any(|code| code == "working_tree_clean"));
assert_eq!(json["data"]["risk_hint"]["level"], "low");
assert!(json["data"]["risk_hint"]["factors"].as_array().expect("risk factors").is_empty());
```

In `inspect_reports_merging_during_conflict`, assert:

```rust
assert_eq!(json["data"]["summary"]["state"], "blocked");
assert_eq!(json["data"]["risk_hint"]["level"], "high");
assert!(json["data"]["risk_hint"]["factors"]
    .as_array()
    .expect("risk factors")
    .iter()
    .any(|factor| factor["code"] == "conflicts_present"));
```

- [ ] **Step 2: Run focused tests to verify failure**

Run:

```bash
cargo test -p super-git-cli --test inspect inspect_clean_repo_reports_branch_and_no_operation inspect_reports_merging_during_conflict
```

Expected: FAIL because `summary` and `risk_hint` do not exist yet.

- [ ] **Step 3: Add models**

In `model.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectSummary {
    pub state: String,
    pub codes: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RiskFactor {
    pub code: String,
    pub level: RiskLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectRiskHint {
    pub level: RiskLevel,
    pub factors: Vec<RiskFactor>,
}
```

Update `RepoState`:

```rust
pub summary: InspectSummary,
pub risk_hint: InspectRiskHint,
```

- [ ] **Step 4: Compute summary and risk**

Add pure helpers:

```rust
fn compute_summary(
    operation: Operation,
    working_tree: &WorkingTree,
    upstream: &Option<UpstreamInfo>,
    worktree_context: &WorktreeContext,
) -> InspectSummary {
    let mut codes = Vec::new();
    let state = if working_tree.conflict_count > 0 {
        "blocked"
    } else if operation != Operation::None {
        "in_progress"
    } else if !working_tree.clean {
        "dirty"
    } else {
        "ready"
    };

    codes.push(match operation {
        Operation::None => "operation_none".to_string(),
        Operation::Merging => "operation_merging".to_string(),
        Operation::Rebasing => "operation_rebasing".to_string(),
        Operation::Applying => "operation_applying".to_string(),
        Operation::CherryPicking => "operation_cherry_picking".to_string(),
        Operation::Reverting => "operation_reverting".to_string(),
        Operation::Bisecting => "operation_bisecting".to_string(),
    });

    codes.push(if working_tree.clean {
        "working_tree_clean".to_string()
    } else {
        "working_tree_dirty".to_string()
    });

    if working_tree.conflict_count > 0 {
        codes.push("conflicts_present".to_string());
    }

    codes.push(match upstream {
        None => "upstream_none".to_string(),
        Some(u) if u.ahead == 0 && u.behind == 0 => "upstream_synced".to_string(),
        Some(u) if u.ahead > 0 && u.behind == 0 => "upstream_ahead".to_string(),
        Some(u) if u.ahead == 0 && u.behind > 0 => "upstream_behind".to_string(),
        Some(_) => "upstream_diverged".to_string(),
    });

    codes.push(match worktree_context.kind {
        WorktreeKind::Main => "main_worktree".to_string(),
        WorktreeKind::Linked => "linked_worktree".to_string(),
        WorktreeKind::Bare => "bare_worktree".to_string(),
        WorktreeKind::Unknown => "unknown_worktree".to_string(),
    });

    let message = match state {
        "blocked" => "Resolve conflicts before continuing Git operations.",
        "in_progress" => "A Git operation is in progress.",
        "dirty" => "Working tree has local changes.",
        _ => "Repository is ready for a safe next action.",
    }
    .to_string();

    InspectSummary {
        state: state.to_string(),
        codes,
        message,
    }
}

fn compute_inspect_risk_hint(operation: Operation, working_tree: &WorkingTree) -> InspectRiskHint {
    let mut factors = Vec::new();
    if working_tree.conflict_count > 0 {
        factors.push(RiskFactor {
            code: "conflicts_present".to_string(),
            level: RiskLevel::High,
            message: "Unmerged paths must be resolved before continuing.".to_string(),
        });
    }
    if operation != Operation::None {
        factors.push(RiskFactor {
            code: "operation_in_progress".to_string(),
            level: RiskLevel::Medium,
            message: "Repository is inside an in-progress Git operation.".to_string(),
        });
    }

    let level = if factors.iter().any(|f| f.level == RiskLevel::High) {
        RiskLevel::High
    } else if factors.iter().any(|f| f.level == RiskLevel::Medium) {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    InspectRiskHint { level, factors }
}
```

- [ ] **Step 5: Render JSON and compact human output**

In JSON output, include:

```rust
"summary": state.summary,
"risk_hint": state.risk_hint,
```

In human output, add one line near the top:

```rust
println!("Summary: {} ({})", state.summary.state, state.summary.message);
println!("Risk hint: {}", risk_level_label(state.risk_hint.level));
```

- [ ] **Step 6: Run focused and full tests**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/super-git-core/src/model.rs crates/super-git-core/src/git/state.rs crates/super-git-cli/src/output.rs crates/super-git-cli/tests/inspect.rs
git commit -m "feat(inspect): add summary and current-state risk hint"
```

---

### Task 3: Replace `allowed_next` with `next` Guardrail Buckets

**Files:**
- Modify: `crates/super-git-core/src/model.rs`
- Modify: `crates/super-git-core/src/git/state.rs`
- Modify: `crates/super-git-cli/src/output.rs`
- Modify: `crates/super-git-cli/tests/inspect.rs`
- Modify: `README.md`
- Modify: `docs/setup.md`
- Modify: `docs/architecture.md`
- Modify: `docs/roadmap.md`

- [ ] **Step 1: Add failing helper and assertions**

Replace `action_kinds` in `crates/super-git-cli/tests/inspect.rs`:

```rust
fn next_kinds(json: &serde_json::Value, bucket: &str) -> Vec<String> {
    json["data"]["next"][bucket]
        .as_array()
        .unwrap_or_else(|| panic!("next.{bucket} array"))
        .iter()
        .map(|a| a["kind"].as_str().expect("kind").to_string())
        .collect()
}
```

Update existing assertions from:

```rust
action_kinds(&json)
```

to:

```rust
next_kinds(&json, "allowed")
```

Add focused blocked assertions:

```rust
let blocked = next_kinds(&json, "blocked");
assert!(blocked.iter().any(|k| k == "continue_operation"));
```

for conflict tests, and:

```rust
assert!(json["data"]["allowed_next"].is_null());
assert_eq!(json["data"]["schema_version"], "super-git.inspect.v0.2");
```

to pin removal of the old field.

- [ ] **Step 2: Run inspect tests to verify failure**

Run:

```bash
cargo test -p super-git-cli --test inspect
```

Expected: FAIL because `next` does not exist, `allowed_next` still exists, and the schema version is still v0.1.

- [ ] **Step 3: Add NextGuardrails model**

In `model.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextGuardrails {
    pub allowed: Vec<NextAction>,
    pub blocked: Vec<NextAction>,
    pub needs_human_review: Vec<NextAction>,
}
```

Update `RepoState`:

```rust
pub next: NextGuardrails,
```

Remove `allowed_next` from `RepoState`.

- [ ] **Step 4: Compute allowed, blocked, and human-review buckets**

Rename `compute_allowed_next` to `compute_next_guardrails` and return `NextGuardrails`.

Keep the current allowed logic as `allowed`.

When this task changes the canonical action schema from `allowed_next` to `next`, bump the inspect schema version from:

```rust
pub const INSPECT_SCHEMA_VERSION: &str = "super-git.inspect.v0.1";
```

to:

```rust
pub const INSPECT_SCHEMA_VERSION: &str = "super-git.inspect.v0.2";
```

Add blocked actions only for state-specific traps:

```rust
fn compute_blocked_actions(
    operation: Operation,
    working_tree: &WorkingTree,
    upstream: &Option<UpstreamInfo>,
) -> Vec<NextAction> {
    let mut blocked = Vec::new();
    if working_tree.conflict_count > 0 {
        blocked.push(action(
            "continue_operation",
            "conflicts remain; resolve all unmerged paths before continuing",
            None,
            None,
        ));
    }
    if operation == Operation::None && working_tree.staged == 0 && (working_tree.unstaged > 0 || working_tree.untracked > 0) {
        blocked.push(action(
            "commit",
            "changes are not staged yet",
            None,
            None,
        ));
    }
    if operation == Operation::None && !working_tree.clean {
        if let Some(u) = upstream {
            if u.behind > 0 {
                blocked.push(action(
                    "pull",
                    "working tree has local changes; clean or stage/commit before integrating upstream",
                    Some(&["git", "pull"]),
                    None,
                ));
            }
            if u.ahead > 0 && u.behind > 0 {
                blocked.push(action(
                    "integrate_diverged",
                    "working tree has local changes; clean or stage/commit before rebasing/merging upstream",
                    Some(&["git", "pull", "--rebase"]),
                    None,
                ));
            }
        }
    }
    blocked
}
```

Keep `needs_human_review` empty for now:

```rust
needs_human_review: Vec::new(),
```

This avoids pretending inspect has an exhaustive global dangerous-command list.

Normalize JSON action `kind` values to snake_case in this task. Human output may still use prose labels, but machine IDs should not mix CLI kebab-case with JSON snake_case before C4 consumes them.

- [ ] **Step 5: Render JSON and human output**

JSON output should include:

```rust
"next": state.next,
```

and must remove:

```rust
"allowed_next": state.allowed_next,
```

Human output should print:

```rust
if !state.next.allowed.is_empty() {
    println!("Next allowed:");
    for next in &state.next.allowed {
        println!("  - {} ({})", next.kind, next.reason);
    }
}
if !state.next.blocked.is_empty() {
    println!("Next blocked:");
    for next in &state.next.blocked {
        println!("  - {} ({})", next.kind, next.reason);
    }
}
```

- [ ] **Step 6: Update docs**

Update the `inspect` example and descriptions in:

- `README.md`
- `docs/setup.md`
- `docs/architecture.md`
- `docs/roadmap.md`

Docs must state:

- `inspect` has a versioned schema.
- `next.allowed` is safe preview candidates, not permission to execute raw Git commands.
- `next.blocked` is not an exhaustive global denylist; it is current-state traps.
- `next.needs_human_review` is reserved for actions where preview may be possible but automatic execute should not proceed without explicit human approval.

- [ ] **Step 7: Run verification**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
/Users/hokkk/ai-friendly/super-git-home/super-git/target/debug/super-git inspect | jq '.data | {schema_version, summary, risk_hint, upstream, warnings, next}'
```

Expected:

- format clean
- clippy clean
- all tests pass
- dogfood inspect prints no `allowed_next`
- dogfood inspect prints `next.allowed`, `next.blocked`, `next.needs_human_review`
- JSON action IDs use snake_case.

- [ ] **Step 8: Commit**

```bash
git add crates/super-git-core/src/model.rs crates/super-git-core/src/git/state.rs crates/super-git-cli/src/output.rs crates/super-git-cli/tests/inspect.rs README.md docs/setup.md docs/architecture.md docs/roadmap.md
git commit -m "feat(inspect): replace allowed_next with next guardrails"
```

---

## Self-Review Notes

- This plan intentionally avoids `profile`, `guide`, dashboard, preview, execute, and undo implementation.
- `inspect.risk_hint` is current-state risk only. It must not describe the risk of a future command; C4 preview owns action risk.
- `warnings` are factual context, not moral judgment. The first warning only states that upstream comparison is based on local tracking refs.
- `next.allowed` means preview candidate, not execute permission.
- `next.blocked` is not a global list of every dangerous Git command. It only contains state-specific traps where preconditions are not currently met.
- `needs_human_review` exists in the schema but can be empty until preview/execute introduces actions that require explicit approval.
- C4 preview must create its own plan risk and its own stronger fingerprint. The current inspect summary counts are not sufficient as an execution lock.
