//! `super-git undo`의 첫 복구 계약 통합 테스트.
//! stage_changes undo는 index snapshot만 복원하고 working tree 파일 내용은 바꾸지 않아야 한다.

use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
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

fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = run_git(dir, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git stdout utf8")
}

fn path_string(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

fn worktree_list(dir: &Path) -> String {
    git_stdout(dir, &["worktree", "list", "--porcelain"])
}

fn preview_plan_file(dir: &Path) -> PathBuf {
    let output = super_git(dir)
        .args(["preview", "stage-changes"])
        .output()
        .expect("run preview");
    assert!(
        output.status.success(),
        "preview failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let plan_path = dir.join(".git").join("super-git-test-plan.json");
    std::fs::write(&plan_path, output.stdout).expect("write plan");
    plan_path
}

fn preview_worktree_plan_file(dir: &Path, app_home: &Path, ref_name: &str) -> (PathBuf, PathBuf) {
    let output = super_git(dir)
        .args(["preview", "worktree-create", "--ref", ref_name])
        .env("SUPER_GIT_HOME", app_home)
        .output()
        .expect("run worktree preview");
    assert!(
        output.status.success(),
        "worktree preview failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("preview json");
    let target = PathBuf::from(
        json["data"]["target"]["path"]
            .as_str()
            .expect("target path"),
    );

    let plan_path = dir.join(".git").join("super-git-test-worktree-plan.json");
    std::fs::write(&plan_path, output.stdout).expect("write worktree plan");
    (plan_path, target)
}

fn execute_plan(dir: &Path, plan: &Path) -> serde_json::Value {
    let output = super_git(dir)
        .args(["execute", "--plan"])
        .arg(plan)
        .output()
        .expect("run execute");
    assert!(
        output.status.success(),
        "execute failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).expect("parse execute json")
}

fn execute_worktree_plan(dir: &Path, app_home: &Path, plan: &Path) -> serde_json::Value {
    let output = super_git(dir)
        .args(["execute", "--plan"])
        .arg(plan)
        .env("SUPER_GIT_HOME", app_home)
        .output()
        .expect("run worktree execute");
    assert!(
        output.status.success(),
        "worktree execute failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).expect("parse execute json")
}

fn write_token_file(dir: &Path, execute_output: &serde_json::Value) -> PathBuf {
    let token_path = dir.join(".git").join("super-git-test-token.json");
    std::fs::write(
        &token_path,
        serde_json::to_vec_pretty(execute_output).expect("serialize token"),
    )
    .expect("write token");
    token_path
}

fn registry_path_from_execute_output(execute_output: &serde_json::Value) -> PathBuf {
    let mut path = PathBuf::from(
        execute_output["data"]["undo_token"]["index_snapshot_path"]
            .as_str()
            .expect("snapshot path"),
    );
    path.set_extension("json");
    path
}

fn undo_token(dir: &Path, token: &Path) -> Output {
    super_git(dir)
        .args(["undo", "--token"])
        .arg(token)
        .output()
        .expect("run undo")
}

fn setup_worktree_create(
    tmp: &tempfile::TempDir,
    branch: &str,
) -> (PathBuf, PathBuf, serde_json::Value, PathBuf) {
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", branch]);
    let (plan, target) = preview_worktree_plan_file(&repo, &app_home, branch);
    let execute_output = execute_worktree_plan(&repo, &app_home, &plan);
    let token = write_token_file(&repo, &execute_output);
    (repo, target, execute_output, token)
}

#[test]
fn undo_stage_changes_fails_when_registry_record_is_missing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    std::fs::remove_file(registry_path_from_execute_output(&execute_output))
        .expect("remove registry record");

    let output = undo_token(dir, &token);

    assert!(
        !output.status.success(),
        "undo should reject tokens without registry provenance"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("registry_missing")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[test]
fn undo_stage_changes_fails_when_registry_record_token_differs() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    let registry_path = registry_path_from_execute_output(&execute_output);
    let mut registry: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&registry_path).expect("read registry"))
            .expect("parse registry");
    registry["undo_token"]["plan_id"] = serde_json::json!("sha256:tampered");
    std::fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("serialize registry"),
    )
    .expect("write registry");

    let output = undo_token(dir, &token);

    assert!(
        !output.status.success(),
        "undo should reject registry/token mismatch"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("registry_token_mismatch")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[test]
fn undo_stage_changes_fails_when_registry_token_hash_differs() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    let registry_path = registry_path_from_execute_output(&execute_output);
    let mut registry: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&registry_path).expect("read registry"))
            .expect("parse registry");
    registry["token_sha256"] = serde_json::json!("sha256:tampered");
    std::fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("serialize registry"),
    )
    .expect("write registry");

    let output = undo_token(dir, &token);

    assert!(
        !output.status.success(),
        "undo should reject registry hash mismatch"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("registry.token_sha256")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[cfg(unix)]
#[test]
fn undo_stage_changes_rejects_registry_symlink() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    let registry_path = registry_path_from_execute_output(&execute_output);
    let target = dir.join(".git").join("fake-registry.json");
    std::fs::copy(&registry_path, &target).expect("copy registry target");
    std::fs::remove_file(&registry_path).expect("remove registry");
    symlink(&target, &registry_path).expect("replace registry with symlink");

    let output = undo_token(dir, &token);

    assert!(!output.status.success(), "registry symlink should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("unsafe_registry_file")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[test]
fn undo_stage_changes_rejects_malformed_registry_json() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    std::fs::write(registry_path_from_execute_output(&execute_output), "{")
        .expect("write malformed registry");

    let output = undo_token(dir, &token);

    assert!(!output.status.success(), "malformed registry should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("registry_json_invalid")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[test]
fn undo_stage_changes_rejects_registry_unknown_fields() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    let registry_path = registry_path_from_execute_output(&execute_output);
    let mut registry: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&registry_path).expect("read registry"))
            .expect("parse registry");
    registry["unexpected"] = serde_json::json!(true);
    std::fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("serialize registry"),
    )
    .expect("write registry");

    let output = undo_token(dir, &token);

    assert!(
        !output.status.success(),
        "registry unknown fields should fail"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("registry_json_invalid")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[test]
fn undo_stage_changes_restores_index_without_touching_worktree_files() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    std::fs::write(dir.join("new-file.txt"), "new\n").expect("write untracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    assert_eq!(status_porcelain(dir), "M  file.txt\nA  new-file.txt\n");

    let output = undo_token(dir, &token);

    assert!(
        output.status.success(),
        "undo failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["action"], "stage_changes");
    assert_eq!(json["data"]["undone"], true);
    assert_eq!(status_porcelain(dir), " M file.txt\n?? new-file.txt\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("file.txt")).expect("read file"),
        "hello\nchanged\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("new-file.txt")).expect("read new file"),
        "new\n"
    );
}

#[test]
fn undo_stage_changes_preserves_worktree_mutations_after_execute() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    std::fs::write(dir.join("new-file.txt"), "new\n").expect("write untracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    std::fs::write(dir.join("file.txt"), "hello\nchanged again\n").expect("modify after execute");
    std::fs::remove_file(dir.join("new-file.txt")).expect("remove after execute");

    let output = undo_token(dir, &token);

    assert!(
        output.status.success(),
        "undo failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(status_porcelain(dir), " M file.txt\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("file.txt")).expect("read file"),
        "hello\nchanged again\n"
    );
    assert!(!dir.join("new-file.txt").exists());
}

#[test]
fn undo_stage_changes_fails_when_index_changed_after_execute() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    std::fs::write(dir.join("other.txt"), "other\n").expect("write other");
    git(dir, &["add", "other.txt"]);

    let output = undo_token(dir, &token);

    assert!(
        !output.status.success(),
        "undo should fail after index drift"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("post_index_sha256")));
    assert_eq!(status_porcelain(dir), "M  file.txt\nA  other.txt\n");
}

#[test]
fn undo_stage_changes_accepts_token_from_stdin() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token_bytes = serde_json::to_vec(&execute_output).expect("serialize token");
    let mut child = super_git(dir)
        .args(["undo", "--token", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn undo");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(&token_bytes)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait undo");

    assert!(
        output.status.success(),
        "undo failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(status_porcelain(dir), " M file.txt\n");
}

#[test]
fn undo_stage_changes_accepts_raw_undo_token() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let raw_token = execute_output["data"]["undo_token"].clone();
    let token = write_token_file(dir, &raw_token);

    let output = undo_token(dir, &token);

    assert!(
        output.status.success(),
        "undo failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(status_porcelain(dir), " M file.txt\n");
}

#[test]
fn undo_stage_changes_restores_absent_pre_execute_index_by_removing_index() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("new-file.txt"), "new\n").expect("write untracked");
    assert!(
        !dir.join(".git").join("index").exists(),
        "index starts absent"
    );
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    assert!(
        dir.join(".git").join("index").exists(),
        "execute creates index"
    );

    let output = undo_token(dir, &token);

    assert!(
        output.status.success(),
        "undo failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !dir.join(".git").join("index").exists(),
        "undo removes absent pre-index"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("new-file.txt")).expect("read new file"),
        "new\n"
    );
}

#[test]
fn undo_stage_changes_rejects_repository_tampering() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let mut execute_output = execute_plan(dir, &plan);
    execute_output["data"]["undo_token"]["repository"] = serde_json::json!("/tmp/not-this-repo");
    let token = write_token_file(dir, &execute_output);

    let output = undo_token(dir, &token);

    assert!(!output.status.success(), "tampered repository should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause.as_str().unwrap_or_default().contains("repository")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[test]
fn undo_stage_changes_rejects_snapshot_path_outside_undo_dir() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let mut execute_output = execute_plan(dir, &plan);
    execute_output["data"]["undo_token"]["index_snapshot_path"] =
        serde_json::json!(tmp.path().join("outside.index"));
    let token = write_token_file(dir, &execute_output);

    let output = undo_token(dir, &token);

    assert!(!output.status.success(), "unsafe snapshot path should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("unsafe_snapshot_path")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[test]
fn undo_stage_changes_rejects_tampered_snapshot_bytes() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    let snapshot = PathBuf::from(
        execute_output["data"]["undo_token"]["index_snapshot_path"]
            .as_str()
            .expect("snapshot path"),
    );
    std::fs::write(snapshot, "tampered").expect("tamper snapshot");

    let output = undo_token(dir, &token);

    assert!(!output.status.success(), "tampered snapshot should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("pre_index_sha256")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
    assert!(
        !dir.join(".git").join("index.lock").exists(),
        "snapshot validation failure must not leave index.lock"
    );
}

#[cfg(unix)]
#[test]
fn undo_stage_changes_rejects_snapshot_symlink() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    let snapshot = PathBuf::from(
        execute_output["data"]["undo_token"]["index_snapshot_path"]
            .as_str()
            .expect("snapshot path"),
    );
    let target = dir.join(".git").join("fake.index");
    std::fs::write(&target, "fake").expect("write fake target");
    std::fs::remove_file(&snapshot).expect("remove snapshot");
    symlink(&target, &snapshot).expect("create snapshot symlink");

    let output = undo_token(dir, &token);

    assert!(!output.status.success(), "snapshot symlink should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("unsafe_snapshot_file")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[test]
fn undo_stage_changes_ignores_ambient_git_index_file() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    let alternate_index = tmp.path().join("alternate-index");

    let output = super_git(dir)
        .args(["undo", "--token"])
        .arg(&token)
        .env("GIT_INDEX_FILE", &alternate_index)
        .output()
        .expect("run undo");

    assert!(
        output.status.success(),
        "undo failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(status_porcelain(dir), " M file.txt\n");
    assert!(
        !alternate_index.exists(),
        "undo must not write through ambient GIT_INDEX_FILE"
    );
}

#[test]
fn undo_stage_changes_fails_when_index_lock_exists() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let execute_output = execute_plan(dir, &plan);
    let token = write_token_file(dir, &execute_output);
    std::fs::write(dir.join(".git").join("index.lock"), "locked").expect("create lock");

    let output = undo_token(dir, &token);

    assert!(
        !output.status.success(),
        "undo should fail while index is locked"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause.as_str().unwrap_or_default().contains("index.lock")));
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[test]
fn undo_worktree_create_removes_clean_linked_worktree_and_preserves_branch() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let branch = "feature/undo-clean";
    let (repo, target, execute_output, token) = setup_worktree_create(&tmp, branch);
    let branch_ref = format!("refs/heads/{branch}");
    let branch_head_before = git_stdout(&repo, &["rev-parse", &branch_ref])
        .trim()
        .to_string();
    let parent = execute_output["data"]["undo_token"]["created_parent"]
        .as_str()
        .map(PathBuf::from)
        .expect("created parent");
    assert!(
        target.join(".git").exists(),
        "target worktree starts present"
    );
    assert!(
        worktree_list(&repo).contains(&path_string(&target)),
        "target starts as an active worktree"
    );

    let output = undo_token(&repo, &token);

    assert!(
        output.status.success(),
        "undo failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["action"], "worktree_create");
    assert_eq!(json["data"]["undone"], true);
    assert!(
        !worktree_list(&repo).contains(&path_string(&target)),
        "target must no longer be an active worktree"
    );
    assert_eq!(
        git_stdout(&repo, &["rev-parse", &branch_ref])
            .trim()
            .to_string(),
        branch_head_before,
        "undo must preserve the branch ref and commit"
    );
    assert!(
        !parent.exists(),
        "undo should remove an empty parent directory created by super-git"
    );
}

#[test]
fn undo_worktree_create_rejects_dirty_target_without_removing_it() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let (repo, target, _execute_output, token) = setup_worktree_create(&tmp, "feature/undo-dirty");
    std::fs::write(target.join("untracked.txt"), "do not remove me\n").expect("dirty target");

    let output = undo_token(&repo, &token);

    assert!(
        !output.status.success(),
        "undo should reject a dirty target worktree"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("target_working_tree_clean")));
    assert!(
        worktree_list(&repo).contains(&path_string(&target)),
        "dirty target must remain active"
    );
    assert!(
        target.join("untracked.txt").exists(),
        "dirty file must remain untouched"
    );
}

#[test]
fn undo_worktree_create_rejects_ignored_target_files_without_removing_them() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let (repo, target, _execute_output, token) =
        setup_worktree_create(&tmp, "feature/undo-ignored");
    std::fs::write(
        repo.join(".git").join("info").join("exclude"),
        "ignored.log\n",
    )
    .expect("write git exclude");
    std::fs::write(target.join("ignored.log"), "do not remove me\n").expect("write ignored file");

    let output = undo_token(&repo, &token);

    assert!(
        !output.status.success(),
        "undo should reject ignored files before removing a target worktree"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("target_working_tree_clean_including_ignored")));
    assert!(
        worktree_list(&repo).contains(&path_string(&target)),
        "target with ignored files must remain active"
    );
    assert!(
        target.join("ignored.log").exists(),
        "ignored file must remain untouched"
    );
}

#[test]
fn undo_worktree_create_rejects_locked_target_without_removing_it() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let (repo, target, _execute_output, token) = setup_worktree_create(&tmp, "feature/undo-locked");
    git(&repo, &["worktree", "lock", target.to_str().unwrap()]);

    let output = undo_token(&repo, &token);

    let _ = run_git(&repo, &["worktree", "unlock", target.to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "undo should reject a locked target worktree"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause.as_str().unwrap_or_default().contains("target_locked")));
    assert!(
        worktree_list(&repo).contains(&path_string(&target)),
        "locked target must remain active"
    );
}

#[test]
fn undo_worktree_create_rejects_execution_record_tampering() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let (repo, target, execute_output, token) =
        setup_worktree_create(&tmp, "feature/undo-record-tamper");
    let record_path = PathBuf::from(
        execute_output["data"]["undo_token"]["execution_record_path"]
            .as_str()
            .expect("execution record path"),
    );
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record_path).expect("read record"))
            .expect("parse record");
    record["undo_token"]["plan_id"] = serde_json::json!("sha256:tampered");
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&record).expect("serialize record"),
    )
    .expect("write tampered record");

    let output = undo_token(&repo, &token);

    assert!(
        !output.status.success(),
        "undo should reject record/token mismatch"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("execution_record_token_mismatch")));
    assert!(
        worktree_list(&repo).contains(&path_string(&target)),
        "target must remain active after provenance failure"
    );
}

#[test]
fn undo_worktree_create_accepts_raw_undo_token() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let branch = "feature/undo-raw";
    let (repo, target, execute_output, _token) = setup_worktree_create(&tmp, branch);
    let raw_token = execute_output["data"]["undo_token"].clone();
    let token = write_token_file(&repo, &raw_token);

    let output = undo_token(&repo, &token);

    assert!(
        output.status.success(),
        "undo failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !worktree_list(&repo).contains(&path_string(&target)),
        "raw worktree token should remove the linked worktree"
    );
}

#[test]
fn undo_worktree_create_rejects_incomplete_execution_record() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let (repo, target, execute_output, token) =
        setup_worktree_create(&tmp, "feature/undo-incomplete-record");
    let record_path = PathBuf::from(
        execute_output["data"]["undo_token"]["execution_record_path"]
            .as_str()
            .expect("execution record path"),
    );
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record_path).expect("read record"))
            .expect("parse record");
    record["status"] = serde_json::json!("intent");
    record["undo_token"] = serde_json::Value::Null;
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&record).expect("serialize record"),
    )
    .expect("write incomplete record");

    let output = undo_token(&repo, &token);

    assert!(
        !output.status.success(),
        "undo should reject incomplete execution records"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("execution_record_incomplete")));
    assert!(
        worktree_list(&repo).contains(&path_string(&target)),
        "target must remain active when record is incomplete"
    );
}

#[test]
fn undo_worktree_create_rejects_target_branch_drift() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let (repo, target, _execute_output, token) =
        setup_worktree_create(&tmp, "feature/undo-branch-drift");
    git(&target, &["checkout", "-q", "--detach"]);

    let output = undo_token(&repo, &token);

    assert!(
        !output.status.success(),
        "undo should reject target branch drift"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause.as_str().unwrap_or_default().contains("target_branch")));
    assert!(
        worktree_list(&repo).contains(&path_string(&target)),
        "target must remain active after branch drift refusal"
    );
}
