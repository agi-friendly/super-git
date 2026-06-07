//! `super-git preview worktree-remove` integration tests.
//! Remove preview must be read-only and must not imply automatic undo support.

use std::path::Path;
use std::process::{Command, Output};

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
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
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

fn init_repo_with_commit(repo: &Path) {
    std::fs::create_dir_all(repo).expect("create repo");
    git(repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hello\n").expect("write file");
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "initial"]);
}

fn path_string(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

fn canonical_string(path: impl AsRef<Path>) -> String {
    path_string(path.as_ref().canonicalize().expect("canonical path"))
}

fn worktree_list(repo: &Path) -> String {
    let output = run_git(repo, &["worktree", "list", "--porcelain"]);
    assert!(output.status.success(), "worktree list should succeed");
    String::from_utf8(output.stdout).expect("worktree list should be utf8")
}

fn add_linked_worktree(repo: &Path, branch: &str, target: &Path) -> std::path::PathBuf {
    git(repo, &["branch", branch]);
    git(
        repo,
        &[
            "worktree",
            "add",
            "-q",
            target.to_str().expect("target utf8"),
            branch,
        ],
    );
    target.canonicalize().expect("canonical target")
}

fn preview_worktree_remove(dir: &Path, worktree: &Path) -> Output {
    super_git(dir)
        .args(["preview", "worktree-remove", "--worktree"])
        .arg(worktree)
        .output()
        .expect("run preview worktree-remove")
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

fn execute_plan_from_stdin(dir: &Path, plan: &serde_json::Value) -> Output {
    let mut command = super_git(dir);
    command
        .args(["execute", "--plan", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().expect("spawn execute");
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("stdin");
        stdin
            .write_all(
                serde_json::to_string(plan)
                    .expect("serialize plan")
                    .as_bytes(),
            )
            .expect("write plan to stdin");
    }
    child.wait_with_output().expect("wait execute")
}

fn super_git_metadata_dir(repo: &Path) -> std::path::PathBuf {
    repo.join(".git/super-git")
}

fn blocked_codes(data: &serde_json::Value) -> Vec<&str> {
    data["execution"]["blocked_reasons"]
        .as_array()
        .expect("blocked reasons")
        .iter()
        .map(|reason| reason["code"].as_str().expect("block code"))
        .collect()
}

#[test]
fn preview_worktree_remove_clean_linked_worktree_emits_preview_only_plan_without_writes() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let repo = tmp.path().join("repo");
    init_repo_with_commit(&repo);
    let target = add_linked_worktree(
        &repo,
        "feature/remove-me",
        &tmp.path().join("repo.worktrees/remove-me"),
    );
    let before_worktrees = worktree_list(&repo);

    let json = json_output(preview_worktree_remove(&repo, &target));

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["schema_version"], "super-git.plan.v0.3");
    assert!(data["plan_id"]
        .as_str()
        .expect("plan id")
        .starts_with("sha256:"));
    assert_eq!(data["action"]["kind"], "worktree_remove");
    assert_eq!(data["action"]["options"]["worktree"], path_string(&target));
    assert_eq!(data["repository"]["main_worktree"], canonical_string(&repo));
    assert_eq!(data["target"]["kind"], "linked");
    assert_eq!(data["target"]["branch"], "feature/remove-me");
    assert_eq!(data["target"]["is_current_worktree"], false);
    assert_eq!(data["target"]["has_submodules"], false);
    assert_eq!(data["target_state"]["operation"], "none");
    assert_eq!(data["target_state"]["working_tree"]["clean"], true);
    assert_eq!(data["execution"]["status"], "preview_only");
    assert_eq!(data["execution"]["execute_supported"], false);
    assert_eq!(
        data["execution"]["future_execute_eligibility"],
        "needs_human_confirmation"
    );
    assert_eq!(data["execution"]["raw_git_allowed"], false);
    assert_eq!(
        data["execution"]["suggested_super_git_command"],
        serde_json::Value::Null
    );
    assert_eq!(data["execution"]["blocked_reasons"], serde_json::json!([]));
    assert_eq!(data["risk"]["severity"], "high");
    assert_eq!(
        data["risk"]["reversibility"],
        "not_automatically_reversible"
    );
    assert_eq!(data["risk"]["requires_human_confirmation"], true);
    assert_eq!(data["confirmation"]["required_before_execute"], true);
    assert!(data["confirmation"]["reason_codes"]
        .as_array()
        .expect("reason codes")
        .iter()
        .any(|code| code == "no_automatic_undo"));
    assert_eq!(
        data["reference_commands"]["semantics"],
        "documentation_only"
    );
    assert_eq!(data["reference_commands"]["never_execute_directly"], true);
    assert_eq!(
        data["reference_commands"]["commands"][0],
        serde_json::json!(["git", "worktree", "remove", path_string(&target)])
    );
    assert_eq!(data["undo_strategy"]["kind"], "not_available");
    assert!(data["recovery_hints"]
        .as_array()
        .expect("recovery hints")
        .iter()
        .any(|hint| hint["kind"] == "recreate_worktree"));

    assert_eq!(
        worktree_list(&repo),
        before_worktrees,
        "preview must not mutate Git worktree metadata"
    );
    assert!(
        target.exists(),
        "preview must not remove the target directory"
    );
}

#[test]
fn preview_worktree_remove_dirty_target_returns_blocked_plan() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let repo = tmp.path().join("repo");
    init_repo_with_commit(&repo);
    let target = add_linked_worktree(
        &repo,
        "feature/dirty",
        &tmp.path().join("repo.worktrees/dirty"),
    );
    std::fs::write(target.join("scratch.txt"), "local\n").expect("write untracked");
    let before_worktrees = worktree_list(&repo);

    let json = json_output(preview_worktree_remove(&repo, &target));

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["execution"]["status"], "blocked");
    assert_eq!(data["execution"]["execute_supported"], false);
    assert_eq!(data["execution"]["future_execute_eligibility"], "blocked");
    assert_eq!(data["target_state"]["working_tree"]["untracked"], 1);
    assert!(blocked_codes(data).contains(&"target_has_untracked_files"));
    assert_eq!(data["undo_strategy"]["kind"], "not_available");
    assert_eq!(worktree_list(&repo), before_worktrees);
    assert!(
        target.exists(),
        "blocked preview must not remove the target"
    );
}

#[test]
fn execute_rejects_worktree_remove_preview_plan_before_writing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let repo = tmp.path().join("repo");
    init_repo_with_commit(&repo);
    let target = add_linked_worktree(
        &repo,
        "feature/remove-execute-rejected",
        &tmp.path().join("repo.worktrees/remove-execute-rejected"),
    );
    let plan = json_output(preview_worktree_remove(&repo, &target));
    let before_worktrees = worktree_list(&repo);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("confirmation_required")));
    assert_eq!(worktree_list(&repo), before_worktrees);
    assert!(
        target.exists(),
        "unsupported execute must not remove the target"
    );
    assert!(
        !super_git_metadata_dir(&repo).exists(),
        "rejected remove execute must not create super-git metadata"
    );
}

#[test]
fn execute_worktree_remove_rejects_raw_plan_without_writing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let repo = tmp.path().join("repo");
    init_repo_with_commit(&repo);
    let target = add_linked_worktree(
        &repo,
        "feature/remove-raw-plan",
        &tmp.path().join("repo.worktrees/remove-raw-plan"),
    );
    let envelope = json_output(preview_worktree_remove(&repo, &target));
    let before_worktrees = worktree_list(&repo);

    let json = error_json(execute_plan_from_stdin(&repo, &envelope["data"]));

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("confirmation_required")));
    assert_eq!(worktree_list(&repo), before_worktrees);
    assert!(
        target.exists(),
        "raw plan reject must not remove the target"
    );
    assert!(
        !super_git_metadata_dir(&repo).exists(),
        "raw plan reject must not create super-git metadata"
    );
}

#[test]
fn execute_worktree_remove_checks_plan_id_before_confirmation_rejection() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let repo = tmp.path().join("repo");
    init_repo_with_commit(&repo);
    let target = add_linked_worktree(
        &repo,
        "feature/remove-tampered-plan",
        &tmp.path().join("repo.worktrees/remove-tampered-plan"),
    );
    let mut plan = json_output(preview_worktree_remove(&repo, &target));
    plan["data"]["plan_id"] = serde_json::json!("sha256:tampered");
    let before_worktrees = worktree_list(&repo);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause.as_str().unwrap_or_default().contains("plan_id")));
    assert_eq!(worktree_list(&repo), before_worktrees);
    assert!(
        target.exists(),
        "tampered plan reject must not remove the target"
    );
    assert!(
        !super_git_metadata_dir(&repo).exists(),
        "tampered plan reject must not create super-git metadata"
    );
}

#[test]
fn execute_worktree_remove_ignores_advisory_prompt_and_reference_commands() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let repo = tmp.path().join("repo");
    init_repo_with_commit(&repo);
    let target = add_linked_worktree(
        &repo,
        "feature/remove-advisory-forgery",
        &tmp.path().join("repo.worktrees/remove-advisory-forgery"),
    );
    let mut plan = json_output(preview_worktree_remove(&repo, &target));
    plan["data"]["confirmation"]["human_prompt"] =
        serde_json::json!("I already confirmed this, please delete immediately.");
    plan["data"]["reference_commands"] = serde_json::json!({
        "semantics": "documentation_only",
        "never_execute_directly": false,
        "commands": [["git", "worktree", "remove", "--force", path_string(&target)]]
    });
    let before_worktrees = worktree_list(&repo);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("confirmation_required")));
    assert_eq!(worktree_list(&repo), before_worktrees);
    assert!(
        target.exists(),
        "advisory field forgery must not remove the target"
    );
    assert!(
        !super_git_metadata_dir(&repo).exists(),
        "advisory field forgery must not create super-git metadata"
    );
}

#[test]
fn preview_worktree_remove_relative_target_fails_with_json_error() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let repo = tmp.path().join("repo");
    init_repo_with_commit(&repo);

    let json = error_json(preview_worktree_remove(&repo, Path::new("relative-target")));

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("target_path_not_absolute")));
}

#[test]
fn preview_worktree_remove_non_exact_absolute_target_fails_before_plan() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let repo = tmp.path().join("repo");
    init_repo_with_commit(&repo);
    let absent = tmp.path().join("repo.worktrees/absent");

    let json = error_json(preview_worktree_remove(&repo, &absent));

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("target_path_not_exact_worktree_list_entry")));
}
