//! `super-git undo` integration tests for `history_edit` results (C8-D).
//! Undo restores the branch tip via compare-and-swap and touches nothing else.

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
        .env("GIT_AUTHOR_NAME", "Jane Author")
        .env("GIT_AUTHOR_EMAIL", "jane@example.com")
        .env("GIT_AUTHOR_DATE", "2026-06-09T10:00:00+09:00")
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

fn commit_file(repo: &Path, file: &str, content: &str, message: &str) {
    std::fs::write(repo.join(file), content).expect("write file");
    git(repo, &["add", file]);
    git(repo, &["commit", "-q", "-m", message]);
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

fn pipe_stdin(mut command: Command, input: &str) -> Output {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait")
}

fn preview_plan(dir: &Path, base: &str, instructions: &str) -> serde_json::Value {
    let mut command = super_git(dir);
    command.args([
        "preview",
        "history-edit",
        "--base",
        base,
        "--instructions",
        "-",
    ]);
    let output = pipe_stdin(command, instructions);
    assert!(
        output.status.success(),
        "preview failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse preview json")
}

fn execute(dir: &Path, plan: &serde_json::Value) -> serde_json::Value {
    let mut command = super_git(dir);
    command.args(["execute", "--plan", "-"]);
    let output = pipe_stdin(command, &serde_json::to_string(plan).expect("plan json"));
    assert!(
        output.status.success(),
        "execute failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse execute json")
}

fn undo(dir: &Path, result: &serde_json::Value) -> Output {
    let mut command = super_git(dir);
    command.args(["undo", "--token", "-"]);
    pipe_stdin(
        command,
        &serde_json::to_string(result).expect("result json"),
    )
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

fn reword_fixup_items(oids: &[String]) -> String {
    instructions_doc(&format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"reworded"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    ))
}

#[test]
fn execute_then_undo_restores_exact_pre_edit_tip() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let refs_before = git_stdout(&repo, &["for-each-ref"]);

    let plan = preview_plan(&repo, "main", &reword_fixup_items(&oids));
    let result = execute(&repo, &plan);
    assert_ne!(
        git_stdout(&repo, &["rev-parse", "HEAD"]),
        tip_before,
        "execute should move the tip"
    );

    let json = json_output(undo(&repo, &result));

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["action"], "history_edit");
    assert_eq!(json["data"]["undone"], true);
    // The branch tip is bit-for-bit the original; the whole ref set matches.
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
    assert_eq!(git_stdout(&repo, &["for-each-ref"]), refs_before);
    // The original commit ids are reachable again.
    let restored: Vec<String> = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(restored, oids);
    // HEAD stays attached and the working tree is clean.
    assert_eq!(
        git_stdout(&repo, &["symbolic-ref", "HEAD"]),
        "refs/heads/feature/login"
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain=v1"]), "");
}

#[test]
fn undo_refuses_when_branch_advanced_after_execute() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &reword_fixup_items(&oids));
    let result = execute(&repo, &plan);
    // A new commit after execute means undo can no longer safely reclaim.
    commit_file(&repo, "d.txt", "d\n", "later work");
    let tip_after_advance = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(undo(&repo, &result));

    assert_eq!(json["ok"], false);
    assert!(cause_contains(&json, "branch_advanced_since_execute"));
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD"]),
        tip_after_advance,
        "refused undo must not move the branch"
    );
}

#[test]
fn undo_refuses_when_execution_record_is_deleted() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &reword_fixup_items(&oids));
    let result = execute(&repo, &plan);
    let tip_after_execute = git_stdout(&repo, &["rev-parse", "HEAD"]);
    // Provenance is the local execution record; without it, undo refuses.
    let execution_dir = repo.join(".git/super-git/executions");
    for entry in std::fs::read_dir(&execution_dir).expect("execution dir") {
        std::fs::remove_file(entry.expect("entry").path()).expect("remove record");
    }

    let json = error_json(undo(&repo, &result));

    assert_eq!(json["ok"], false);
    assert!(cause_contains(&json, "execution_record_missing"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_after_execute);
}

#[test]
fn undo_refuses_token_with_tampered_previous_tip() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &reword_fixup_items(&oids));
    let mut result = execute(&repo, &plan);
    let tip_after_execute = git_stdout(&repo, &["rev-parse", "HEAD"]);
    // Forge the restore target: the embedded execution record no longer matches.
    result["data"]["undo_token"]["previous_tip"] =
        serde_json::json!("0000000000000000000000000000000000000000");

    let json = error_json(undo(&repo, &result));

    assert_eq!(json["ok"], false);
    assert!(cause_contains(&json, "execution_record_token_mismatch"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_after_execute);
}

#[test]
fn undo_is_idempotent_refusing_a_second_run() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let plan = preview_plan(&repo, "main", &reword_fixup_items(&oids));
    let result = execute(&repo, &plan);

    json_output(undo(&repo, &result));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);

    // The branch is back at previous_tip, no longer at new_tip, so a replay is
    // rejected rather than moving the branch a second time.
    let json = error_json(undo(&repo, &result));
    assert!(cause_contains(&json, "branch_advanced_since_execute"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

// ---- C8-drop-C: drop undo stays fail-closed until C8-drop-D ----

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

#[test]
fn drop_undo_token_fails_closed_until_c8_drop_d() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = instructions_doc(&format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    ));
    let plan = preview_plan(&repo, "main", &items);
    // Artifacts live outside the repo: drop's clean-tree gate counts untracked.
    let artifacts = repo.parent().expect("repo has a parent dir");
    let plan_file = artifacts.join("plan.json");
    let confirmation_file = artifacts.join("confirmation.json");
    std::fs::write(&plan_file, serde_json::to_vec(&plan).expect("plan")).expect("write plan");
    std::fs::write(
        &confirmation_file,
        serde_json::to_vec(&confirmation_for(&plan)).expect("confirmation"),
    )
    .expect("write confirmation");
    let output = super_git(&repo)
        .arg("execute")
        .arg("--plan")
        .arg(&plan_file)
        .arg("--confirmation")
        .arg(&confirmation_file)
        .output()
        .expect("run execute");
    let result = serde_json::from_slice::<serde_json::Value>(&output.stdout).expect("json");
    assert_eq!(result["ok"], true, "drop execute must succeed: {result}");
    let tip_after_execute = git_stdout(&repo, &["rev-parse", "HEAD"]);

    // restore_branch_tip_and_worktree lands in C8-drop-D; until then the undo
    // surface must refuse the new kind instead of half-restoring (ref without
    // working tree). Nothing may move on refusal.
    let json = error_json(undo(&repo, &result));

    assert!(
        cause_contains(&json, "unsupported_undo_kind"),
        "drop undo must fail closed: {json}"
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_after_execute);
    assert!(
        !repo.join("b.txt").exists(),
        "refused undo must not resurrect the dropped patch"
    );
}
