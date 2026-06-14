//! `super-git execute` confirmation-gated tests for published `history_edit`
//! ranges (C8-E). Rewriting published history requires a matching
//! `super-git.confirmation.v0.1` artifact and fresh revalidation.

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
    assert!(output.status.success(), "git {args:?} failed");
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

/// A feature branch whose commits are all reachable from a remote-tracking ref,
/// so the published scan marks the range as published.
fn published_feature_repo(temp: &Path) -> (std::path::PathBuf, Vec<String>) {
    let repo = temp.join("repo");
    init_repo(&repo);
    git(&repo, &["checkout", "-q", "-b", "feature/login"]);
    commit_file(&repo, "a.txt", "a\n", "feat(login): add form");
    commit_file(&repo, "b.txt", "b\n", "fix typo");
    commit_file(&repo, "c.txt", "c\n", "wip");
    let oids: Vec<String> = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    // No network: a local remote-tracking ref is enough for published detection.
    git(
        &repo,
        &["update-ref", "refs/remotes/origin/feature/login", &oids[2]],
    );
    (repo, oids)
}

fn instructions_doc(items: &str) -> String {
    format!(
        r#"{{"schema_version":"super-git.instructions.v0.1","action":"history_edit","base":"main","items":{items}}}"#
    )
}

fn reword_items(oids: &[String]) -> String {
    instructions_doc(&format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"reworded"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    ))
}

fn alternate_reword_items(oids: &[String]) -> String {
    instructions_doc(&format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"alternate reword"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    ))
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

/// Builds the deterministic, valid confirmation for a published plan envelope.
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

fn write_json(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_vec(value).expect("serialize")).expect("write json");
}

fn execute_with_confirmation(
    dir: &Path,
    plan: &serde_json::Value,
    confirmation: &serde_json::Value,
) -> Output {
    let plan_file = dir.join("plan.json");
    let confirmation_file = dir.join("confirmation.json");
    write_json(&plan_file, plan);
    write_json(&confirmation_file, confirmation);
    super_git(dir)
        .arg("execute")
        .arg("--plan")
        .arg(&plan_file)
        .arg("--confirmation")
        .arg(&confirmation_file)
        .output()
        .expect("run execute")
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

#[test]
fn published_plan_with_valid_confirmation_rewrites_and_yields_undo_token() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = published_feature_repo(tmp.path());
    let head_tree_before = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
    let plan = preview_plan(&repo, "main", &reword_items(&oids));
    assert_eq!(plan["data"]["execution"]["status"], "preview_only");
    let confirmation = confirmation_for(&plan);

    let json = json_output(execute_with_confirmation(&repo, &plan, &confirmation));

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["action"], "history_edit");
    assert_eq!(
        json["data"]["undo_token"]["kind"],
        "restore_branch_tip_snapshot"
    );
    // The reword landed and the final tree is preserved.
    let subjects = git_stdout(&repo, &["log", "--reverse", "--format=%s", "main..HEAD"]);
    assert!(subjects.contains("reworded"));
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]),
        head_tree_before
    );
}

#[test]
fn published_plan_without_confirmation_is_rejected() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = published_feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &reword_items(&oids));
    let plan_file = repo.join("plan.json");
    write_json(&plan_file, &plan);
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let output = super_git(&repo)
        .arg("execute")
        .arg("--plan")
        .arg(&plan_file)
        .output()
        .expect("run execute");
    let json = error_json(output);

    assert!(cause_contains(&json, "confirmation_required"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn published_plan_with_wrong_phrase_is_rejected() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = published_feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &reword_items(&oids));
    let mut confirmation = confirmation_for(&plan);
    confirmation["acknowledgement"]["phrase"] =
        serde_json::json!("rewrite published history on whatever at deadbeef");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_with_confirmation(&repo, &plan, &confirmation));

    assert!(cause_contains(&json, "confirmation_phrase_mismatch"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn published_confirmation_phrase_is_bound_to_the_exact_plan() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = published_feature_repo(tmp.path());
    let plan_a = preview_plan(&repo, "main", &reword_items(&oids));
    let plan_b = preview_plan(&repo, "main", &alternate_reword_items(&oids));
    assert_ne!(plan_a["data"]["plan_id"], plan_b["data"]["plan_id"]);
    assert_ne!(
        plan_a["data"]["confirmation"]["required_phrase"],
        plan_b["data"]["confirmation"]["required_phrase"]
    );
    let mut confirmation = confirmation_for(&plan_b);
    confirmation["acknowledgement"]["phrase"] =
        plan_a["data"]["confirmation"]["required_phrase"].clone();
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_with_confirmation(&repo, &plan_b, &confirmation));

    assert!(cause_contains(&json, "confirmation_phrase_mismatch"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn published_plan_with_mismatched_target_is_rejected() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = published_feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &reword_items(&oids));
    let mut confirmation = confirmation_for(&plan);
    confirmation["target"]["branch_ref"] = serde_json::json!("refs/heads/some-other-branch");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_with_confirmation(&repo, &plan, &confirmation));

    assert!(cause_contains(&json, "confirmation_target_mismatch"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn published_plan_with_unsupported_confirmation_schema_is_rejected() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = published_feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &reword_items(&oids));
    let mut confirmation = confirmation_for(&plan);
    confirmation["schema_version"] = serde_json::json!("super-git.confirmation.v999");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_with_confirmation(&repo, &plan, &confirmation));

    assert!(cause_contains(&json, "confirmation_schema_unsupported"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn unpublished_plan_rejects_an_unexpected_confirmation() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = published_feature_repo(tmp.path());
    // Drop the remote-tracking ref so the range is no longer published.
    git(
        &repo,
        &["update-ref", "-d", "refs/remotes/origin/feature/login"],
    );
    let plan = preview_plan(&repo, "main", &reword_items(&oids));
    assert_eq!(plan["data"]["execution"]["status"], "executable");
    // Forge a confirmation for a plan that does not need one.
    let confirmation = serde_json::json!({
        "schema_version": "super-git.confirmation.v0.1",
        "kind": "destructive_action_confirmation",
        "action": "history_edit"
    });
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_with_confirmation(&repo, &plan, &confirmation));

    assert!(cause_contains(&json, "confirmation_not_supported"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn published_rewrite_can_be_undone() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = published_feature_repo(tmp.path());
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let plan = preview_plan(&repo, "main", &reword_items(&oids));
    let confirmation = confirmation_for(&plan);
    let result = json_output(execute_with_confirmation(&repo, &plan, &confirmation));
    assert_ne!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);

    // The undo token works the same for published rewrites: it restores the
    // local tip but does not (and cannot) un-publish the remote ref.
    let result_file = repo.join("result.json");
    write_json(&result_file, &result);
    let undo = json_output(
        super_git(&repo)
            .arg("undo")
            .arg("--token")
            .arg(&result_file)
            .output()
            .expect("run undo"),
    );
    assert_eq!(undo["data"]["undone"], true);
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}
