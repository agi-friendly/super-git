//! `super-git execute`의 첫 write 계약 통합 테스트.
//! execute는 preview plan을 다시 검증한 뒤 내부 allowlist로만 Git을 실행해야 한다.

use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use sha2::{Digest, Sha256};
use super_git_core::git::preview::compute_plan_id;
use super_git_core::model::{Operation, PreviewPlan, UndoRegistryRecord};

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

fn preview_plan_file(dir: &Path) -> PathBuf {
    let output = super_git(dir)
        .args(["preview", "stage-changes"])
        .output()
        .expect("run preview");
    assert!(
        output.status.success(),
        "preview failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let plan_path = dir.join(".git").join("super-git-test-plan.json");
    std::fs::write(&plan_path, output.stdout).expect("write plan");
    plan_path
}

fn preview_plan_envelope(dir: &Path) -> serde_json::Value {
    let output = super_git(dir)
        .args(["preview", "stage-changes"])
        .output()
        .expect("run preview");
    assert!(
        output.status.success(),
        "preview failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse preview json")
}

fn write_plan_envelope(dir: &Path, envelope: &serde_json::Value) -> PathBuf {
    let plan_path = dir.join(".git").join("super-git-test-plan.json");
    std::fs::write(
        &plan_path,
        serde_json::to_vec_pretty(envelope).expect("serialize plan"),
    )
    .expect("write plan");
    plan_path
}

fn execute_plan(dir: &Path, plan: &Path) -> Output {
    super_git(dir)
        .args(["execute", "--plan"])
        .arg(plan)
        .output()
        .expect("run execute")
}

fn execute_plan_with_confirmation(dir: &Path, plan: &Path, confirmation: &Path) -> Output {
    super_git(dir)
        .arg("execute")
        .arg("--plan")
        .arg(plan)
        .arg("--confirmation")
        .arg(confirmation)
        .output()
        .expect("run execute with confirmation")
}

fn status_porcelain(dir: &Path) -> String {
    let output = run_git(dir, &["status", "--porcelain=v1"]);
    assert!(output.status.success(), "status should succeed");
    String::from_utf8_lossy(&output.stdout).to_string()
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

fn registry_path_from_plan(dir: &Path, plan_envelope: &serde_json::Value) -> PathBuf {
    let plan_id = plan_envelope["data"]["plan_id"].as_str().expect("plan_id");
    let pre_index_sha256 =
        sha256_hex(&std::fs::read(dir.join(".git").join("index")).unwrap_or_default());
    let mut hasher = Sha256::new();
    hasher.update(b"super-git-execute-v0.1\n");
    hasher.update(plan_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(pre_index_sha256.as_bytes());
    let execution_id = format_hex_digest(hasher.finalize().as_slice());
    dir.join(".git")
        .join("super-git")
        .join("undo")
        .join(format!("{execution_id}.json"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", format_hex_digest(hasher.finalize().as_slice()))
}

fn format_hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(hex_char(byte >> 4));
        output.push(hex_char(byte & 0x0f));
    }
    output
}

fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + value - 10) as char,
        _ => unreachable!("nibble is always <= 15"),
    }
}

#[test]
fn execute_stage_changes_rejects_unexpected_confirmation_without_staging() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let confirmation = dir.join(".git").join("unused-confirmation.json");
    std::fs::write(&confirmation, "{}").expect("write confirmation");

    let output = execute_plan_with_confirmation(dir, &plan, &confirmation);

    assert!(
        !output.status.success(),
        "unexpected confirmation should fail"
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
            .contains("confirmation_not_supported")));
    assert_eq!(
        status_porcelain(dir),
        " M file.txt\n",
        "unexpected confirmation must not stage changes"
    );
}

#[test]
fn execute_stage_changes_stages_previewed_paths_and_returns_undo_token() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    std::fs::write(dir.join("new-file.txt"), "new\n").expect("write untracked");
    let plan = preview_plan_file(dir);

    let output = super_git(dir)
        .args(["execute", "--plan"])
        .arg(&plan)
        .output()
        .expect("run execute");

    assert!(
        output.status.success(),
        "execute failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["action"], "stage_changes");
    assert_eq!(json["data"]["undo_token"]["kind"], "restore_index_snapshot");
    assert_eq!(json["data"]["undo_token"]["pre_index_existed"], true);
    assert!(json["data"]["undo_token"]["post_index_sha256"]
        .as_str()
        .unwrap_or_default()
        .starts_with("sha256:"));
    assert!(!json["data"]["undo_token"]["index_snapshot_path"]
        .as_str()
        .expect("snapshot path")
        .contains("sha256:"));
    let registry_path = registry_path_from_execute_output(&json);
    let registry: UndoRegistryRecord =
        serde_json::from_slice(&std::fs::read(&registry_path).expect("read registry record"))
            .expect("parse registry record");
    assert_eq!(registry.schema_version, "super-git.undo-registry.v0.1");
    assert_eq!(
        registry.token_sha256,
        sha256_hex(&serde_json::to_vec(&registry.undo_token).expect("serialize token"))
    );
    assert_eq!(
        serde_json::to_value(&registry.undo_token).expect("serialize registry token"),
        json["data"]["undo_token"]
    );

    let status = status_porcelain(dir);
    assert!(status.contains("M  file.txt"), "{status}");
    assert!(status.contains("A  new-file.txt"), "{status}");
}

#[test]
fn execute_stage_changes_rejects_rehashed_plan_during_in_progress_operation() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let mut envelope = preview_plan_envelope(dir);
    std::fs::write(dir.join(".git").join("BISECT_LOG"), "git bisect start\n")
        .expect("mark bisecting");
    let mut plan: PreviewPlan =
        serde_json::from_value(envelope["data"].clone()).expect("parse raw plan");
    plan.state_fingerprint.operation = Operation::Bisecting;
    plan.plan_id = compute_plan_id(&plan).expect("recompute plan id");
    envelope["data"] = serde_json::to_value(plan).expect("serialize raw plan");
    let plan_path = write_plan_envelope(dir, &envelope);

    let output = execute_plan(dir, &plan_path);

    assert!(!output.status.success(), "operation_none must be enforced");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("operation_none")));
    assert_eq!(status_porcelain(dir), " M file.txt\n");
}

#[test]
fn execute_stage_changes_marks_absent_pre_execute_index_in_undo_token() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("new-file.txt"), "new\n").expect("write untracked");
    let plan = preview_plan_file(dir);

    let output = execute_plan(dir, &plan);

    assert!(
        output.status.success(),
        "execute failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["data"]["undo_token"]["pre_index_existed"], false);
    let registry_path = registry_path_from_execute_output(&json);
    assert!(
        registry_path.exists(),
        "execute must write a registry record even when no pre-index snapshot file exists"
    );
    assert_eq!(status_porcelain(dir), "A  new-file.txt\n");
}

#[test]
fn execute_stage_changes_rolls_back_index_when_registry_write_fails() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let envelope = preview_plan_envelope(dir);
    let plan = write_plan_envelope(dir, &envelope);
    let registry_path = registry_path_from_plan(dir, &envelope);
    std::fs::create_dir_all(registry_path.parent().expect("registry parent"))
        .expect("create registry parent");
    std::fs::create_dir(&registry_path).expect("block registry record path with directory");

    let output = execute_plan(dir, &plan);

    assert!(
        !output.status.success(),
        "registry write failure should fail execute"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert_eq!(
        status_porcelain(dir),
        " M file.txt\n",
        "execute must roll back the index if registry provenance cannot be written"
    );
}

#[cfg(unix)]
#[test]
fn execute_stage_changes_rejects_registry_temp_symlink_without_clobbering_target() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let envelope = preview_plan_envelope(dir);
    let plan = write_plan_envelope(dir, &envelope);
    let registry_path = registry_path_from_plan(dir, &envelope);
    let registry_tmp_path = registry_path.with_extension("json.tmp");
    std::fs::create_dir_all(registry_tmp_path.parent().expect("registry parent"))
        .expect("create registry parent");
    let outside = tempfile::tempdir().expect("outside temp dir");
    let target = outside.path().join("outside-target.txt");
    std::fs::write(&target, "do not clobber\n").expect("write target");
    symlink(&target, &registry_tmp_path).expect("create registry temp symlink");

    let output = execute_plan(dir, &plan);

    assert!(!output.status.success(), "temp symlink should fail execute");
    assert_eq!(
        std::fs::read_to_string(&target).expect("read target"),
        "do not clobber\n",
        "registry temp symlink must not be followed or truncated"
    );
    assert_eq!(
        status_porcelain(dir),
        " M file.txt\n",
        "execute must roll back the index after registry temp symlink failure"
    );
}

#[test]
fn execute_stage_changes_fails_when_plan_is_stale() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let before_status = status_porcelain(dir);

    std::fs::write(dir.join("file.txt"), "hello\nchanged again\n").expect("modify after preview");
    let output = execute_plan(dir, &plan);

    assert!(!output.status.success(), "stale plan should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("state_fingerprint")));
    assert_eq!(
        status_porcelain(dir),
        " M file.txt\n",
        "execute must not stage a stale plan; before was {before_status:?}"
    );
}

#[test]
fn execute_stage_changes_rejects_tampered_plan_id_before_writing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let mut envelope = preview_plan_envelope(dir);
    envelope["data"]["plan_id"] = serde_json::json!("sha256:tampered");
    let plan = write_plan_envelope(dir, &envelope);

    let output = execute_plan(dir, &plan);

    assert!(!output.status.success(), "tampered plan_id should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause.as_str().unwrap_or_default().contains("plan_id")));
    assert_eq!(status_porcelain(dir), " M file.txt\n");
}

#[test]
fn execute_stage_changes_ignores_advisory_reference_commands() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let mut envelope = preview_plan_envelope(dir);
    envelope["data"]["reference_commands"] =
        serde_json::json!([["sh", "-c", "touch pwned"], ["git", "reset", "--hard"]]);
    let plan = write_plan_envelope(dir, &envelope);

    let output = execute_plan(dir, &plan);

    assert!(
        output.status.success(),
        "advisory-only tampering should not change plan identity: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !dir.join("pwned").exists(),
        "execute must not run reference_commands"
    );
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}

#[test]
fn execute_stage_changes_ignores_ambient_git_index_file() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let plan = preview_plan_file(dir);
    let alternate_index = tmp.path().join("alternate-index");

    let output = super_git(dir)
        .args(["execute", "--plan"])
        .arg(&plan)
        .env("GIT_INDEX_FILE", &alternate_index)
        .output()
        .expect("run execute");

    assert!(
        output.status.success(),
        "execute failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
    assert!(
        !alternate_index.exists(),
        "execute must not write through ambient GIT_INDEX_FILE"
    );
}

#[test]
fn execute_stage_changes_rejects_rehashed_unsafe_resolved_path() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let mut envelope = preview_plan_envelope(dir);
    let mut plan: PreviewPlan =
        serde_json::from_value(envelope["data"].clone()).expect("parse raw plan");
    plan.action.resolved_paths = vec!["../outside.txt".to_string()];
    plan.plan_id = compute_plan_id(&plan).expect("recompute plan id");
    envelope["data"] = serde_json::to_value(plan).expect("serialize raw plan");
    let plan_path = write_plan_envelope(dir, &envelope);

    let output = execute_plan(dir, &plan_path);

    assert!(!output.status.success(), "unsafe path should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("unsafe_resolved_path")));
    assert_eq!(status_porcelain(dir), " M file.txt\n");
}

#[test]
fn execute_stage_changes_rejects_rehashed_resolved_path_mismatch() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let mut envelope = preview_plan_envelope(dir);
    let mut plan: PreviewPlan =
        serde_json::from_value(envelope["data"].clone()).expect("parse raw plan");
    plan.action.resolved_paths = vec!["other.txt".to_string()];
    plan.plan_id = compute_plan_id(&plan).expect("recompute plan id");
    envelope["data"] = serde_json::to_value(plan).expect("serialize raw plan");
    let plan_path = write_plan_envelope(dir, &envelope);

    let output = execute_plan(dir, &plan_path);

    assert!(!output.status.success(), "pathset mismatch should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("action.resolved_paths")));
    assert_eq!(status_porcelain(dir), " M file.txt\n");
}

#[test]
fn execute_stage_changes_accepts_plan_from_stdin() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    std::fs::write(dir.join("file.txt"), "hello\nchanged\n").expect("modify tracked");
    let envelope = preview_plan_envelope(dir);
    let plan_bytes = serde_json::to_vec(&envelope).expect("serialize plan");
    let mut child = super_git(dir)
        .args(["execute", "--plan", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn execute");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(&plan_bytes)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait execute");

    assert!(
        output.status.success(),
        "execute failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(status_porcelain(dir), "M  file.txt\n");
}
