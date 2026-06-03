//! `super-git preview`의 출력 계약 통합 테스트.
//! preview는 쓰기 작업 전에 plan만 만들고 저장소를 변경하지 않아야 한다.

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

fn init_repo_with_commit(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("file.txt"), "hello\n").expect("write file");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

fn status_porcelain(dir: &Path) -> String {
    let output = run_git(dir, &["status", "--porcelain=v1"]);
    assert!(output.status.success(), "status should succeed");
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn preview_json(dir: &Path) -> serde_json::Value {
    let output = super_git(dir)
        .args(["preview", "stage-changes"])
        .output()
        .expect("run preview");
    assert!(
        output.status.success(),
        "preview failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse json")
}

fn preview_error_json(dir: &Path) -> serde_json::Value {
    let output = super_git(dir)
        .args(["preview", "stage-changes"])
        .output()
        .expect("run preview");
    assert!(!output.status.success(), "preview should fail");
    serde_json::from_slice(&output.stdout).expect("parse error json")
}

#[test]
fn preview_stage_changes_emits_plan_without_staging() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked file");
    std::fs::write(dir.join("new-file.txt"), "new\n").expect("write untracked file");
    let before_status = status_porcelain(dir);

    let json = preview_json(dir);

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["schema_version"], "super-git.plan.v0.1");
    assert!(data["plan_id"]
        .as_str()
        .expect("plan_id string")
        .starts_with("sha256:"));
    assert_eq!(data["action"]["kind"], "stage_changes");
    assert_eq!(data["action"]["scope"], "all");
    assert_eq!(
        data["action"]["resolved_paths"],
        serde_json::json!(["file.txt", "new-file.txt"])
    );
    assert!(Path::new(data["repository"].as_str().expect("repository")).is_absolute());

    let fingerprint = &data["state_fingerprint"];
    assert_eq!(fingerprint["schema_version"], "super-git.fingerprint.v0.1");
    assert!(fingerprint["head_commit"].is_string());
    assert_eq!(fingerprint["operation"], "none");
    for field in [
        "status_porcelain_v1_z_sha256",
        "staged_diff_sha256",
        "unstaged_diff_sha256",
        "untracked_content_sha256",
    ] {
        assert!(
            fingerprint[field]
                .as_str()
                .unwrap_or_default()
                .starts_with("sha256:"),
            "{field} should be a sha256 hash"
        );
    }

    assert_eq!(
        data["preconditions"],
        serde_json::json!([
            { "code": "operation_none", "status": "passed" },
            { "code": "no_conflicts", "status": "passed" },
            { "code": "has_unstaged_or_untracked_changes", "status": "passed" }
        ])
    );
    assert_eq!(data["risk"]["severity"], "low");
    assert_eq!(data["risk"]["reversibility"], "reversible");
    assert_eq!(data["risk"]["requires_human_confirmation"], false);
    assert_eq!(
        data["reference_commands"],
        serde_json::json!([["git", "add", "--all"]])
    );
    assert_eq!(data["undo_strategy"]["kind"], "restore_index_snapshot");
    assert_eq!(data["undo_strategy"]["requires_index_snapshot"], true);
    assert_eq!(data["undo_preview"]["kind"], "restore_index_snapshot");
    assert_eq!(data["undo_preview"]["available_after_execute"], true);

    assert_eq!(
        status_porcelain(dir),
        before_status,
        "preview must not stage or modify files"
    );
}

#[test]
fn preview_stage_changes_plan_id_is_deterministic_for_same_state() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked file");
    std::fs::write(dir.join("new-file.txt"), "new\n").expect("write untracked file");

    let first = preview_json(dir);
    let second = preview_json(dir);

    assert_eq!(first["data"]["plan_id"], second["data"]["plan_id"]);
}

#[test]
fn preview_stage_changes_fails_when_repo_is_clean() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    let json = preview_error_json(dir);

    assert_eq!(json["ok"], false);
    assert!(json["error"]["message"]
        .as_str()
        .expect("error message")
        .contains("could not preview stage_changes"));
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("has_unstaged_or_untracked_changes")));
}

#[test]
fn preview_stage_changes_fails_when_conflicts_exist() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    git(dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("file.txt"), "feature\n").expect("write feature");
    git(dir, &["commit", "-q", "-am", "feature change"]);
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("file.txt"), "main\n").expect("write main");
    git(dir, &["commit", "-q", "-am", "main change"]);
    let merge = run_git(dir, &["merge", "feature"]);
    assert!(!merge.status.success(), "merge should conflict");

    let json = preview_error_json(dir);

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause.as_str().unwrap_or_default().contains("no_conflicts")));
}

#[test]
fn preview_stage_changes_rejects_pathspecs_for_c4_a() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "changed\n").expect("modify tracked file");

    let output = super_git(dir)
        .args(["preview", "stage-changes", "file.txt"])
        .output()
        .expect("run preview");
    assert!(!output.status.success(), "pathspec should be rejected");

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["message"], "invalid command-line arguments");
}

#[test]
fn preview_stage_changes_rejects_dash_dash_pathspecs_for_c4_a() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "changed\n").expect("modify tracked file");

    let output = super_git(dir)
        .args(["preview", "stage-changes", "--", "file.txt"])
        .output()
        .expect("run preview");
    assert!(!output.status.success(), "pathspec should be rejected");

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["message"], "invalid command-line arguments");
}

#[test]
fn preview_missing_subcommand_fails_with_json_envelope() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    let output = super_git(dir).arg("preview").output().expect("run preview");
    assert!(!output.status.success(), "missing subcommand should fail");

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["message"], "invalid command-line arguments");
}

#[test]
fn preview_unknown_subcommand_fails_with_json_envelope() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    let output = super_git(dir)
        .args(["preview", "nope"])
        .output()
        .expect("run preview");
    assert!(!output.status.success(), "unknown subcommand should fail");

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["message"], "invalid command-line arguments");
}

#[test]
fn preview_stage_changes_from_subdir_still_resolves_repo_root_paths() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    let subdir = dir.join("subdir");
    std::fs::create_dir(&subdir).expect("mkdir subdir");
    std::fs::write(dir.join("root-untracked.txt"), "root\n").expect("write root untracked");
    std::fs::write(subdir.join("nested-untracked.txt"), "nested\n")
        .expect("write nested untracked");

    let json = preview_json(&subdir);

    assert_eq!(
        json["data"]["action"]["resolved_paths"],
        serde_json::json!(["root-untracked.txt", "subdir/nested-untracked.txt"])
    );
}

#[cfg(unix)]
#[test]
fn preview_stage_changes_symlink_fingerprint_tracks_link_target_not_target_contents() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).expect("mkdir repo");
    init_repo_with_commit(&dir);
    let external_a = tmp.path().join("external-a.txt");
    let external_b = tmp.path().join("external-b.txt");
    std::fs::write(&external_a, "first\n").expect("write target");
    symlink(&external_a, dir.join("link.txt")).expect("create symlink");

    let first = preview_json(&dir);
    let first_hash = first["data"]["state_fingerprint"]["untracked_content_sha256"]
        .as_str()
        .expect("hash")
        .to_string();

    std::fs::write(&external_a, "changed target contents\n").expect("modify target");
    let second = preview_json(&dir);
    let second_hash = second["data"]["state_fingerprint"]["untracked_content_sha256"]
        .as_str()
        .expect("hash")
        .to_string();
    assert_eq!(
        first_hash, second_hash,
        "symlink fingerprint must not follow target file contents"
    );

    std::fs::remove_file(dir.join("link.txt")).expect("remove symlink");
    symlink(&external_b, dir.join("link.txt")).expect("replace symlink");
    let third = preview_json(&dir);
    let third_hash = third["data"]["state_fingerprint"]["untracked_content_sha256"]
        .as_str()
        .expect("hash")
        .to_string();
    assert_ne!(
        first_hash, third_hash,
        "symlink fingerprint should change when the link target string changes"
    );
}
