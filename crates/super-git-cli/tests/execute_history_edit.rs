//! `super-git execute` integration tests for `history_edit` plans (C8-C,
//! C8-drop-C). Execute must rebuild commits with plumbing, move the branch
//! ref by compare-and-swap, and verify the final tree. Tree-preserving plans
//! never touch the working tree; drop plans require a clean tree and
//! synchronize it to the new tip afterwards.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn super_git(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_super-git"));
    cmd.current_dir(dir);
    cmd
}

fn run_git(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.com")
        .output()
        .expect("run git")
}

fn git(dir: &Path, args: &[&str]) {
    let output = run_git(dir, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = run_git(dir, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_repo(repo: &Path) {
    std::fs::create_dir_all(repo).expect("create repo");
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "user.name", "committer"]);
    git(repo, &["config", "user.email", "committer@example.com"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
    commit_file(repo, "README.md", "hello\n", "initial");
}

/// Author is pinned per commit so author preservation can be asserted against a
/// committer that differs from the author.
fn commit_file(repo: &Path, file: &str, content: &str, message: &str) {
    std::fs::write(repo.join(file), content).expect("write file");
    git(repo, &["add", file]);
    Command::new("git")
        .current_dir(repo)
        .args(["commit", "-q", "-m", message])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Jane Author")
        .env("GIT_AUTHOR_EMAIL", "jane@example.com")
        .env("GIT_AUTHOR_DATE", "2026-06-09T10:00:00+09:00")
        .env("GIT_COMMITTER_NAME", "committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.com")
        .output()
        .expect("commit");
}

fn feature_repo(temp: &Path) -> (std::path::PathBuf, Vec<String>) {
    let repo = temp.join("repo");
    init_repo(&repo);
    git(&repo, &["checkout", "-q", "-b", "feature/login"]);
    commit_file(&repo, "a.txt", "a\n", "feat(login): add form");
    commit_file(&repo, "b.txt", "b\n", "fix typo");
    commit_file(&repo, "c.txt", "c\n", "wip");
    let oids = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    (repo, oids)
}

fn instructions_doc(items: &str) -> String {
    format!(
        r#"{{"schema_version":"super-git.instructions.v0.1","action":"history_edit","base":"main","items":{items}}}"#
    )
}

fn preview_plan(dir: &Path, base: &str, instructions: &str) -> serde_json::Value {
    let mut command = super_git(dir);
    command
        .args([
            "preview",
            "history-edit",
            "--base",
            base,
            "--instructions",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn preview");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(instructions.as_bytes())
        .expect("write instructions");
    let output = child.wait_with_output().expect("wait preview");
    assert!(
        output.status.success(),
        "preview failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse preview json")
}

fn execute_plan_from_stdin(dir: &Path, plan: &serde_json::Value) -> Output {
    let mut command = super_git(dir);
    command
        .args(["execute", "--plan", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn execute");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(
            serde_json::to_string(plan)
                .expect("serialize plan")
                .as_bytes(),
        )
        .expect("write plan");
    child.wait_with_output().expect("wait execute")
}

fn json_output(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "command failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).expect("parse json")
}

fn error_json(output: Output) -> serde_json::Value {
    assert!(
        !output.status.success(),
        "command should fail: stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).expect("parse error json")
}

fn cause_contains(json: &serde_json::Value, needle: &str) -> bool {
    json["error"]["causes"]
        .as_array()
        .map(|causes| {
            causes
                .iter()
                .any(|cause| cause.as_str().unwrap_or_default().contains(needle))
        })
        .unwrap_or(false)
}

fn subjects(repo: &Path, range: &str) -> Vec<String> {
    git_stdout(repo, &["log", "--reverse", "--format=%s", range])
        .lines()
        .map(str::to_string)
        .collect()
}

/// f.txt에 의존적 편집을 가진 체인: c2와 c3가 같은 줄을 잇달아 바꾼다.
/// c2를 drop하면 c3 replay가 충돌해 plan이 blocked가 된다.
fn dependent_repo(temp: &Path) -> (std::path::PathBuf, Vec<String>) {
    let repo = temp.join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("f.txt"), "a\nb\nc\n").expect("write");
    git(&repo, &["add", "f.txt"]);
    git(&repo, &["commit", "-q", "-m", "add f"]);
    git(&repo, &["checkout", "-q", "-b", "feature/dep"]);
    commit_file(&repo, "a.txt", "a\n", "c1 unrelated");
    std::fs::write(repo.join("f.txt"), "X\nb\nc\n").expect("write");
    git(&repo, &["commit", "-q", "-am", "c2 line1 X"]);
    std::fs::write(repo.join("f.txt"), "Y\nb\nc\n").expect("write");
    git(&repo, &["commit", "-q", "-am", "c3 line1 Y"]);
    let oids = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    (repo, oids)
}

/// Builds the valid confirmation for a confirmation-gated plan by echoing the
/// plan's own reason codes and deterministic required phrase.
fn confirmation_for(plan: &serde_json::Value) -> serde_json::Value {
    let data = &plan["data"];
    serde_json::json!({
        "schema_version": "super-git.confirmation.v0.1",
        "kind": "destructive_action_confirmation",
        "action": "history_edit",
        "plan_schema_version": data["schema_version"],
        "plan_id": data["plan_id"],
        "target": {
            "branch_ref": data["branch"]["ref"],
            "git_common_dir": data["repository"]["git_common_dir"],
            "tip_commit": data["branch"]["tip_commit"]
        },
        "acknowledged_reason_codes": data["confirmation"]["reason_codes"],
        "acknowledged_undo_strategy": data["undo_strategy"]["kind"],
        "acknowledgement": {
            "method": "cli_typed_phrase",
            "phrase": data["confirmation"]["required_phrase"]
        }
    })
}

fn execute_with_confirmation(
    dir: &Path,
    plan: &serde_json::Value,
    confirmation: &serde_json::Value,
) -> Output {
    // Artifacts live outside the repo: drop's clean-tree gate counts untracked
    // files, so writing them into the working tree would block execute.
    let artifacts = dir.parent().expect("repo has a parent dir");
    let plan_file = artifacts.join("plan.json");
    let confirmation_file = artifacts.join("confirmation.json");
    std::fs::write(&plan_file, serde_json::to_vec(plan).expect("plan")).expect("write plan");
    std::fs::write(
        &confirmation_file,
        serde_json::to_vec(confirmation).expect("confirmation"),
    )
    .expect("write confirmation");
    super_git(dir)
        .arg("execute")
        .arg("--plan")
        .arg(&plan_file)
        .arg("--confirmation")
        .arg(&confirmation_file)
        .output()
        .expect("run execute")
}

#[test]
fn reword_and_fixup_rewrites_history_preserving_tree_and_author() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let head_tree_before = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"fix(login): validate email"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    let json = json_output(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["schema_version"], "super-git.execute.v0.2");
    assert_eq!(data["action"], "history_edit");
    assert_eq!(data["executed"], true);
    assert_eq!(data["undo_token"]["kind"], "restore_branch_tip_snapshot");

    // Two commits remain and the messages reflect reword + fixup.
    let after = subjects(&repo, "main..HEAD");
    assert_eq!(
        after,
        vec![
            "feat(login): add form".to_string(),
            "fix(login): validate email".to_string()
        ]
    );
    // The tree at the tip is byte-identical: history edit preserves content.
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]),
        head_tree_before
    );
    // Author identity is preserved even though the committer differs.
    let authors = git_stdout(&repo, &["log", "--format=%an <%ae>", "main..HEAD"]);
    assert!(authors
        .lines()
        .all(|line| line == "Jane Author <jane@example.com>"));
    assert!(git_stdout(&repo, &["log", "-1", "--format=%cn"]) != "Jane Author");
    // HEAD stays attached to the branch; working tree stays clean.
    assert_eq!(
        git_stdout(&repo, &["symbolic-ref", "HEAD"]),
        "refs/heads/feature/login"
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain=v1"]), "");
}

#[test]
fn unchanged_prefix_commit_keeps_its_object_id() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    // First commit is a plain pick, so it must keep its original id; only the
    // reworded commit and everything after it gets new ids.
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"reworded b"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    json_output(execute_plan_from_stdin(&repo, &plan));

    let new_oids: Vec<String> = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(new_oids.len(), 3);
    assert_eq!(new_oids[0], oids[0], "unchanged prefix keeps its object id");
    assert_ne!(new_oids[1], oids[1], "reworded commit gets a new id");
    assert_ne!(new_oids[2], oids[2], "commit after the edit is rebuilt too");
}

#[test]
fn squash_folds_two_commits_into_one_with_explicit_message() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let head_tree_before = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
    // pick a, pick b, squash c into b: c folds into the second commit, leaving two.
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"squash","message":"combined b and c"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    json_output(execute_plan_from_stdin(&repo, &plan));

    let after = subjects(&repo, "main..HEAD");
    assert_eq!(
        after,
        vec![
            "feat(login): add form".to_string(),
            "combined b and c".to_string()
        ]
    );
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]),
        head_tree_before
    );
    // All three files still exist: squash preserves content, only history shape changed.
    for file in ["a.txt", "b.txt", "c.txt"] {
        assert!(repo.join(file).exists(), "{file} should still exist");
    }
}

#[test]
fn old_v0_4_plan_is_rejected_with_a_clear_schema_signal() {
    // The C8-drop series added prediction and drop preconditions to the plan-id
    // projection, so the hash contract moved from v0.4 to v0.5. A plan minted by
    // an older binary must fail with an explicit unsupported_schema_version, not
    // a cryptic plan_id mismatch.
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let mut plan = preview_plan(&repo, "main", &instructions_doc(&items));
    plan["data"]["schema_version"] = serde_json::json!("super-git.plan.v0.4");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["error"]["code"], "unsupported_schema_version");
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn stale_plan_after_new_commit_is_rejected_without_moving_ref() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    // Advance the branch after preview: the plan is now stale.
    commit_file(&repo, "d.txt", "d\n", "later commit");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], false);
    // The reconstructed range no longer matches, so fresh binding rejects it.
    assert!(
        cause_contains(&json, "plan_id") || cause_contains(&json, "fresh_execution_status"),
        "stale plan must be rejected: {json}"
    );
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD"]),
        tip_before,
        "stale execute must not move the branch ref"
    );
}

#[test]
fn tampered_plan_id_is_rejected_without_writing() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let mut plan = preview_plan(&repo, "main", &instructions_doc(&items));
    plan["data"]["plan_id"] = serde_json::json!("sha256:tampered");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], false);
    assert!(cause_contains(&json, "plan_id"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn survey_plan_cannot_be_executed() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, _) = feature_repo(tmp.path());
    let plan = json_output(
        super_git(&repo)
            .args(["preview", "history-edit", "--base", "main"])
            .output()
            .expect("survey"),
    );
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], false);
    assert!(cause_contains(&json, "not_executable"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn reorder_executes_ref_only_and_preserves_dirty_worktree() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[1], oids[0], oids[2]
    );
    std::fs::write(repo.join("a.txt"), "dirty but local\n").expect("dirty tracked file");
    let status_before = git_stdout(&repo, &["status", "--porcelain=v1"]);
    let tree_before = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    assert_eq!(plan["data"]["execution"]["status"], "executable");
    assert_eq!(
        plan["data"]["undo_strategy"]["kind"],
        "restore_branch_tip_snapshot"
    );
    let json = json_output(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], true);
    let tip_after = git_stdout(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(tip_after, tip_before);
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]),
        tree_before
    );
    assert_eq!(
        git_stdout(&repo, &["status", "--porcelain=v1"]),
        status_before,
        "reorder execute is ref-only; it must not clean or sync the worktree"
    );
    assert_eq!(
        json["data"]["undo_token"]["kind"],
        "restore_branch_tip_snapshot"
    );
    assert_eq!(
        subjects(&repo, "main..HEAD"),
        vec![
            "fix typo".to_string(),
            "feat(login): add form".to_string(),
            "wip".to_string()
        ]
    );
}

#[test]
fn reorder_rebuilds_from_the_first_moved_position() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let tree_before = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[2], oids[1]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    json_output(execute_plan_from_stdin(&repo, &plan));

    let new_oids: Vec<String> = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        new_oids[0], oids[0],
        "unchanged prefix before the first moved position keeps its original id"
    );
    assert_ne!(
        new_oids[1], oids[2],
        "the first moved commit is replayed onto a new parent"
    );
    assert_ne!(
        new_oids[2], oids[1],
        "commits after the first moved position are rebuilt too"
    );
    assert_eq!(
        subjects(&repo, "main..HEAD"),
        vec![
            "feat(login): add form".to_string(),
            "wip".to_string(),
            "fix typo".to_string()
        ]
    );
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]),
        tree_before
    );
}

#[test]
fn published_reorder_executes_with_the_standard_history_edit_confirmation() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    git(
        &repo,
        &["update-ref", "refs/remotes/origin/feature/login", &oids[2]],
    );
    let tree_before = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[1], oids[0], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    assert_eq!(plan["data"]["execution"]["status"], "preview_only");
    let plan_id = plan["data"]["plan_id"].as_str().expect("plan id");
    let plan_short = plan_id
        .strip_prefix("sha256:")
        .unwrap_or(plan_id)
        .get(..12)
        .expect("short plan id");
    assert_eq!(
        plan["data"]["confirmation"]["required_phrase"],
        format!("rewrite published history on refs/heads/feature/login at {tip_before} for plan {plan_short}")
    );

    let json = json_output(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));

    assert_eq!(json["ok"], true);
    assert_eq!(
        json["data"]["undo_token"]["kind"],
        "restore_branch_tip_snapshot"
    );
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]),
        tree_before
    );
    assert_eq!(
        subjects(&repo, "main..HEAD"),
        vec![
            "fix typo".to_string(),
            "feat(login): add form".to_string(),
            "wip".to_string()
        ]
    );
}

#[test]
fn tampered_advisory_fields_are_ignored_in_favor_of_authentic_repo_values() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"fix: real"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let mut plan = preview_plan(&repo, "main", &instructions_doc(&items));
    // Author and the picked commit's original message are excluded from plan_id
    // (derivable from the bound object id), so a tampered plan keeps a valid id.
    // Execute must ignore these forged copies and use the authentic repo values.
    plan["data"]["range"]["commits"][0]["author_email"] = serde_json::json!("attacker@evil.com");
    plan["data"]["range"]["commits"][0]["author_name"] = serde_json::json!("Attacker");
    plan["data"]["range"]["commits"][0]["message"] = serde_json::json!("forged subject\n");

    let json = json_output(execute_plan_from_stdin(&repo, &plan));
    assert_eq!(
        json["ok"], true,
        "tampered advisory fields must still validate"
    );

    // The picked (first) commit keeps its real author and real message.
    let first_subject = git_stdout(&repo, &["log", "--reverse", "--format=%s", "main..HEAD"])
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        first_subject, "feat(login): add form",
        "forged pick message must be ignored"
    );
    let authors = git_stdout(&repo, &["log", "--format=%ae", "main..HEAD"]);
    assert!(
        authors.lines().all(|line| line == "jane@example.com"),
        "forged author must be ignored, got: {authors}"
    );
}

#[test]
fn execute_writes_completed_execution_record_with_undo_token() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    json_output(execute_plan_from_stdin(&repo, &plan));

    let execution_dir = repo.join(".git/super-git/executions");
    let records: Vec<_> = std::fs::read_dir(&execution_dir)
        .expect("execution dir")
        .collect::<Result<_, _>>()
        .expect("read records");
    assert_eq!(records.len(), 1, "execute should write one record");
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(records[0].path()).expect("read record"))
            .expect("record json");
    assert_eq!(record["status"], "completed");
    assert_eq!(record["action"], "history_edit");
    assert_eq!(record["branch_ref"], "refs/heads/feature/login");
    assert_eq!(record["undo_token"]["kind"], "restore_branch_tip_snapshot");
    assert_eq!(record["commits_before"], 3);
    assert_eq!(record["commits_after"], 2);
}

// ---- C8-drop-C: drop execute ----

#[test]
fn drop_executes_synchronizes_working_tree_and_lands_on_the_predicted_tree() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    let predicted_tree = plan["data"]["prediction"]["final_tree"]
        .as_str()
        .expect("prediction final_tree")
        .to_string();

    let json = json_output(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["executed"], true);
    assert_eq!(
        data["undo_token"]["kind"], "restore_branch_tip_and_worktree",
        "drop undo must advertise the worktree-restoring kind"
    );

    // The dropped patch vanishes from history and from the working tree.
    assert_eq!(
        subjects(&repo, "main..HEAD"),
        vec!["feat(login): add form".to_string(), "wip".to_string()]
    );
    assert!(
        !repo.join("b.txt").exists(),
        "dropped patch must vanish from the working tree"
    );
    assert!(repo.join("c.txt").exists(), "kept patch must survive");
    // The new tip lands exactly on the prediction's final_tree oracle, and the
    // index/working tree are fully synchronized to it.
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]),
        predicted_tree
    );
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        ""
    );
    assert_eq!(
        git_stdout(&repo, &["symbolic-ref", "HEAD"]),
        "refs/heads/feature/login"
    );
    // The kept pick before the first drop keeps its object id; the replayed
    // commit after the drop is new.
    let new_oids: Vec<String> = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(new_oids[0], oids[0], "unchanged prefix keeps its object id");
    assert_ne!(new_oids[1], oids[2], "replayed commit gets a new id");
    let authors = git_stdout(&repo, &["log", "--format=%an <%ae>", "main..HEAD"]);
    assert!(authors
        .lines()
        .all(|line| line == "Jane Author <jane@example.com>"));

    // The execution record carries the oracle tree and the new undo kind.
    let execution_dir = repo.join(".git/super-git/executions");
    let records: Vec<_> = std::fs::read_dir(&execution_dir)
        .expect("execution dir")
        .collect::<Result<_, _>>()
        .expect("read records");
    assert_eq!(records.len(), 1);
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(records[0].path()).expect("read record"))
            .expect("record json");
    assert_eq!(record["status"], "completed");
    assert_eq!(record["final_tree"], predicted_tree.as_str());
    assert_eq!(
        record["undo_token"]["kind"],
        "restore_branch_tip_and_worktree"
    );
    assert_eq!(record["commits_before"], 3);
    assert_eq!(record["commits_after"], 2);
}

#[test]
fn drop_with_reword_replays_the_kept_commit_with_the_new_message() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"reword","message":"chore: keep c"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    json_output(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));

    assert_eq!(
        subjects(&repo, "main..HEAD"),
        vec![
            "feat(login): add form".to_string(),
            "chore: keep c".to_string()
        ]
    );
    assert!(!repo.join("b.txt").exists());
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        ""
    );
}

#[test]
fn drop_execute_without_confirmation_is_rejected() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert!(cause_contains(&json, "confirmation_required"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
    assert!(repo.join("b.txt").exists(), "nothing may change on refusal");
}

#[test]
fn drop_execute_with_the_published_phrase_is_rejected() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);
    // The most plausible wrong phrase: the published-rewrite phrase instead of
    // the drop phrase. One plan, one phrase — this must not authorize a drop.
    let mut confirmation = confirmation_for(&plan);
    let branch_ref = plan["data"]["branch"]["ref"].as_str().expect("ref");
    let plan_id = plan["data"]["plan_id"].as_str().expect("plan id");
    let plan_short = plan_id
        .strip_prefix("sha256:")
        .unwrap_or(plan_id)
        .get(..12)
        .expect("short plan id");
    confirmation["acknowledgement"]["phrase"] = serde_json::json!(format!(
        "rewrite published history on {branch_ref} at {tip_before} for plan {plan_short}"
    ));

    let json = error_json(execute_with_confirmation(&repo, &plan, &confirmation));

    assert!(cause_contains(&json, "confirmation_phrase_mismatch"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn drop_execute_requires_a_clean_tree_and_succeeds_after_cleanup() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );
    // The plan is built on a dirty tree on purpose: cleanliness is volatile
    // state, so it must gate execute without invalidating the plan itself.
    std::fs::write(repo.join("a.txt"), "local edit\n").expect("dirty edit");
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));
    assert!(
        cause_contains(&json, "working_tree_clean"),
        "tracked edits must refuse: {json}"
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
    assert_eq!(
        std::fs::read_to_string(repo.join("a.txt")).expect("read"),
        "local edit\n",
        "a refused execute must not touch the user's edit"
    );

    // Untracked files count as dirty too: a dropped delete could revive a
    // path over them during sync.
    git(&repo, &["checkout", "-q", "--", "a.txt"]);
    std::fs::write(repo.join("scratch.txt"), "untracked\n").expect("untracked");
    let json = error_json(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));
    assert!(cause_contains(&json, "working_tree_clean"));

    // After cleanup the very same plan executes: the dirty preview did not
    // poison the plan_id (non-volatile precondition design).
    std::fs::remove_file(repo.join("scratch.txt")).expect("cleanup");
    let json = json_output(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));
    assert_eq!(json["ok"], true);
    assert!(!repo.join("b.txt").exists());
}

#[test]
fn drop_with_predicted_conflict_cannot_execute() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = dependent_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    assert_eq!(plan["data"]["execution"]["status"], "blocked");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert!(cause_contains(&json, "not_executable"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn drop_everything_moves_the_branch_to_base() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"drop"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    let phrase = plan["data"]["confirmation"]["required_phrase"]
        .as_str()
        .expect("phrase");
    assert!(phrase.starts_with("drop 3 commit(s) from "));

    json_output(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));

    // Abandoning the whole range: the branch lands on base itself.
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD"]),
        git_stdout(&repo, &["rev-parse", "main"])
    );
    for file in ["a.txt", "b.txt", "c.txt"] {
        assert!(!repo.join(file).exists(), "{file} must vanish");
    }
    assert!(repo.join("README.md").exists(), "base content survives");
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        ""
    );
}

#[test]
fn tampered_prediction_final_tree_is_rejected_without_moving_ref() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let mut plan = preview_plan(&repo, "main", &instructions_doc(&items));
    // The final_tree is the execute oracle; forging it must break the plan_id
    // binding rather than steer the post-verify.
    plan["data"]["prediction"]["final_tree"] =
        serde_json::json!("0000000000000000000000000000000000000000");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));

    assert!(cause_contains(&json, "plan_id"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
    assert!(repo.join("b.txt").exists());
}

/// C8-drop-C safety follow-up의 재현 shape: .gitignore에 등록된 파일이 한
/// 커밋에서 force-add됐다가 다음 커밋에서 삭제된 체인. 삭제 커밋을 drop하면
/// 새 tip이 그 ignored 경로를 tracked로 부활시키므로, 같은 자리에 로컬
/// ignored 파일이 있으면 status 기반 clean gate를 통과한 채 sync가 덮어쓸
/// 수 있다 — collision 검사가 막아야 한다.
fn ignored_revival_repo(temp: &Path) -> (std::path::PathBuf, Vec<String>) {
    let repo = temp.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "committer"]);
    git(&repo, &["config", "user.email", "committer@example.com"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join(".gitignore"), "ignored.txt\nscratch.log\n").expect("gitignore");
    git(&repo, &["add", ".gitignore"]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    git(&repo, &["checkout", "-q", "-b", "feature/login"]);
    std::fs::write(repo.join("ignored.txt"), "tracked precious\n").expect("write");
    git(&repo, &["add", "-f", "ignored.txt"]);
    git(&repo, &["commit", "-q", "-m", "force-add ignored.txt"]);
    commit_file(&repo, "a.txt", "a\n", "unrelated");
    git(&repo, &["rm", "-q", "ignored.txt"]);
    git(&repo, &["commit", "-q", "-m", "delete ignored.txt"]);
    let oids = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    (repo, oids)
}

fn drop_delete_commit_items(oids: &[String]) -> String {
    format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}}]"#,
        oids[0], oids[1], oids[2]
    )
}

#[test]
fn drop_refuses_when_an_ignored_file_squats_on_a_revived_path() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = ignored_revival_repo(tmp.path());
    // The user's local ignored file: invisible to the status-based clean gate.
    std::fs::write(repo.join("ignored.txt"), "LOCAL PRECIOUS DATA\n").expect("local file");
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        "",
        "the ignored file must be invisible to the status gate for this regression to bite"
    );
    let plan = preview_plan(
        &repo,
        "main",
        &instructions_doc(&drop_delete_commit_items(&oids)),
    );
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));

    assert!(
        cause_contains(&json, "ignored_path_collision"),
        "collision must be named: {json}"
    );
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD"]),
        tip_before,
        "refusal must happen before the ref moves"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("ignored.txt")).expect("read"),
        "LOCAL PRECIOUS DATA\n",
        "the local ignored file must survive byte-identical"
    );
}

#[test]
fn drop_allows_ignored_files_off_the_new_tip_paths() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = ignored_revival_repo(tmp.path());
    // A normal ignored file (think node_modules): not tracked in the new tip,
    // so it must not block the drop.
    std::fs::write(repo.join("scratch.log"), "build noise\n").expect("scratch");
    let plan = preview_plan(
        &repo,
        "main",
        &instructions_doc(&drop_delete_commit_items(&oids)),
    );

    let json = json_output(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));

    assert_eq!(json["ok"], true);
    assert_eq!(
        std::fs::read_to_string(repo.join("scratch.log")).expect("read"),
        "build noise\n",
        "non-colliding ignored files survive the sync"
    );
    // Dropping the delete commit revives the tracked file content.
    assert_eq!(
        std::fs::read_to_string(repo.join("ignored.txt")).expect("read"),
        "tracked precious\n"
    );
}

#[test]
fn drop_allows_an_empty_ignored_directory_on_a_revived_path() {
    // 정책(C8-drop-D에서 고정): 빈 ignored 디렉터리는 막지 않는다. ls-files
    // -o에 나타나지 않고, 내용물이 없어 잃을 데이터가 없으며, read-tree -u가
    // git checkout과 같은 의미론으로 제거하고 tracked 파일을 실체화한다
    // (스파이크 실측). 내용이 생기는 순간 그 안의 파일들이 prefix 규칙에
    // 걸려 hard block된다 — 바로 위의 디렉터리 충돌 테스트가 그 경우다.
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = ignored_revival_repo(tmp.path());
    std::fs::create_dir(repo.join("ignored.txt")).expect("empty ignored dir");
    let plan = preview_plan(
        &repo,
        "main",
        &instructions_doc(&drop_delete_commit_items(&oids)),
    );

    let json = json_output(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));

    assert_eq!(json["ok"], true);
    assert!(
        repo.join("ignored.txt").is_file(),
        "the empty directory is replaced by the revived tracked file"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("ignored.txt")).expect("read"),
        "tracked precious\n"
    );
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        ""
    );
}

#[test]
fn drop_refuses_when_an_ignored_directory_squats_on_a_revived_file() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = ignored_revival_repo(tmp.path());
    // D/F collision: an ignored directory occupies the path the new tip wants
    // as a tracked file.
    std::fs::create_dir(repo.join("ignored.txt")).expect("mkdir");
    std::fs::write(repo.join("ignored.txt/note.md"), "inside\n").expect("write");
    let plan = preview_plan(
        &repo,
        "main",
        &instructions_doc(&drop_delete_commit_items(&oids)),
    );
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));

    assert!(cause_contains(&json, "ignored_path_collision"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
    assert_eq!(
        std::fs::read_to_string(repo.join("ignored.txt/note.md")).expect("read"),
        "inside\n",
        "the ignored directory must survive untouched"
    );
}

#[test]
fn stale_drop_plan_after_new_commit_is_rejected() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    commit_file(&repo, "d.txt", "d\n", "later commit");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));

    // A moved tip changes the fresh range and the fresh replay prediction;
    // whichever check fires first, nothing may move.
    assert!(
        cause_contains(&json, "fresh_prediction")
            || cause_contains(&json, "plan_id")
            || cause_contains(&json, "fresh_execution_status"),
        "stale drop plan must be rejected: {json}"
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

/// A repo split into an `inside/` cone and an `outside/` tree, with the middle
/// commit (inside/b.txt) droppable. Returns (repo, oids oldest-first).
fn sparse_repo(temp: &Path) -> (std::path::PathBuf, Vec<String>) {
    let repo = temp.join("repo");
    init_repo(&repo);
    git(&repo, &["checkout", "-q", "-b", "feature/login"]);
    std::fs::create_dir_all(repo.join("inside")).expect("mkdir inside");
    std::fs::create_dir_all(repo.join("outside")).expect("mkdir outside");
    std::fs::write(repo.join("inside/keep.txt"), "keep\n").expect("write");
    std::fs::write(repo.join("outside/x.txt"), "x\n").expect("write");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "seed inside and outside"]);
    // c1 (kept) touches the outside tree; c2 (dropped) and c3 (kept) the cone.
    commit_file(&repo, "outside/y.txt", "y\n", "outside change");
    commit_file(&repo, "inside/b.txt", "b\n", "inside b to drop");
    commit_file(&repo, "inside/c.txt", "c\n", "inside c to keep");
    let oids = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    (repo, oids)
}

#[test]
fn drop_execute_respects_the_sparse_checkout_cone() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = sparse_repo(tmp.path());
    // Restrict the working tree to the `inside/` cone before previewing.
    git(&repo, &["sparse-checkout", "set", "inside"]);
    assert!(
        !repo.join("outside/x.txt").exists(),
        "the cone must hide outside paths before the drop"
    );
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        "",
        "a sparse checkout is clean, so the drop gate passes"
    );

    // Drop the middle commit (inside/b.txt); c1 (outside) and c3 (inside) stay.
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2], oids[3]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    let json = json_output(execute_with_confirmation(
        &repo,
        &plan,
        &confirmation_for(&plan),
    ));
    assert_eq!(json["ok"], true);

    // In-cone effects materialize: the dropped file is gone, the kept one stays.
    assert!(
        !repo.join("inside/b.txt").exists(),
        "the dropped in-cone patch must vanish from the working tree"
    );
    assert!(repo.join("inside/c.txt").exists());
    // The cone is respected: kept outside paths are tracked in the new tip but
    // never materialized on disk, and they keep their skip-worktree bit.
    assert!(!repo.join("outside/x.txt").exists());
    assert!(!repo.join("outside/y.txt").exists());
    let tracked = git_stdout(&repo, &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(
        tracked.contains("outside/y.txt"),
        "kept outside path stays tracked"
    );
    assert!(
        !tracked.contains("inside/b.txt"),
        "dropped path left the tree"
    );
    let skipped = git_stdout(&repo, &["ls-files", "-t"]);
    assert!(
        skipped
            .lines()
            .any(|line| line.starts_with("S ") && line.contains("outside/")),
        "outside paths keep skip-worktree, not silently lost: {skipped}"
    );
    // The sync ends clean: no spurious dirty/untracked from the cone.
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        ""
    );
}
