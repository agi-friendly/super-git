//! `super-git predict merge`의 출력 계약 통합 테스트.
//! 핵심 계약: 예측된 충돌은 성공(`ok:true` + status "conflicted")이고,
//! 예측 불가 입력만 structured error다. plan_id/execute/undo는 없다.

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

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).expect("write file");
}

/// main에서 갈라진 left/right 두 브랜치를 만들고 HEAD는 left에 둔다.
fn repo_with_branches(dir: &Path, left_content: &str, right_content: &str) {
    git(dir, &["init", "-q", "-b", "main"]);
    write(dir, "f.txt", "a\nb\nc\nd\ne\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "init"]);
    git(dir, &["checkout", "-q", "-b", "right"]);
    write(dir, "f.txt", right_content);
    git(dir, &["commit", "-q", "-am", "right"]);
    git(dir, &["checkout", "-q", "main"]);
    git(dir, &["checkout", "-q", "-b", "left"]);
    write(dir, "f.txt", left_content);
    git(dir, &["commit", "-q", "-am", "left"]);
}

fn predict_json(dir: &Path, args: &[&str]) -> (serde_json::Value, bool) {
    let output = super_git(dir)
        .args(["predict", "merge"])
        .args(args)
        .output()
        .expect("run predict merge");
    let json = serde_json::from_slice(&output.stdout).expect("parse predict json");
    (json, output.status.success())
}

#[test]
fn clean_merge_is_ok_true_with_clean_status() {
    let tmp = tempfile::tempdir().expect("temp dir");
    repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "a\nb\nc\nd\nRIGHT\n");

    let (json, success) = predict_json(tmp.path(), &["--ours", "left", "--theirs", "right"]);

    assert!(success);
    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["schema_version"], "super-git.conflict-prediction.v0.1");
    assert_eq!(data["prediction_kind"], "merge");
    assert_eq!(data["prediction"]["status"], "clean");
    assert!(data["prediction"]["conflicted_files"]
        .as_array()
        .expect("conflicted_files array")
        .is_empty());
    assert!(data["inputs"]["merge_base"].is_string());
    // plan 계약이 아니다: plan_id도 execute/undo 표면도 없어야 한다.
    let object = data.as_object().expect("data object");
    assert!(!object.contains_key("plan_id"));
    assert!(!object.contains_key("undo_token"));
    assert!(!data["limitations"]
        .as_array()
        .expect("limitations array")
        .is_empty());
}

#[test]
fn predicted_conflict_is_still_ok_true() {
    let tmp = tempfile::tempdir().expect("temp dir");
    repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "RIGHT\nb\nc\nd\ne\n");

    let (json, success) = predict_json(tmp.path(), &["--ours", "left", "--theirs", "right"]);

    // 충돌 예측은 성공한 예측이다 — exit 0, ok:true.
    assert!(success);
    assert_eq!(json["ok"], true);
    let prediction = &json["data"]["prediction"];
    assert_eq!(prediction["status"], "conflicted");
    let files = prediction["conflicted_files"]
        .as_array()
        .expect("conflicted_files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "f.txt");
    let stages: Vec<u64> = files[0]["stages"]
        .as_array()
        .expect("stages array")
        .iter()
        .map(|stage| stage["stage"].as_u64().expect("stage number"))
        .collect();
    assert_eq!(stages, vec![1, 2, 3]);
    assert!(prediction["notes"]
        .as_array()
        .expect("notes array")
        .iter()
        .any(|note| note["kind"] == "CONFLICT (contents)"));
}

#[test]
fn ours_defaults_to_head() {
    let tmp = tempfile::tempdir().expect("temp dir");
    // HEAD는 left에 있다(repo_with_branches가 left를 마지막에 checkout).
    repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "RIGHT\nb\nc\nd\ne\n");

    let (json, success) = predict_json(tmp.path(), &["--theirs", "right"]);

    assert!(success);
    assert_eq!(json["data"]["inputs"]["ours"]["rev"], "HEAD");
    let left_oid =
        String::from_utf8(run_git(tmp.path(), &["rev-parse", "left"]).stdout).expect("utf8 oid");
    assert_eq!(
        json["data"]["inputs"]["ours"]["commit"],
        left_oid.trim(),
        "default ours must resolve to the current HEAD commit"
    );
    assert_eq!(json["data"]["prediction"]["status"], "conflicted");
}

#[test]
fn unknown_theirs_is_structured_rev_not_found() {
    let tmp = tempfile::tempdir().expect("temp dir");
    repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "a\nb\nc\nd\nRIGHT\n");

    let (json, success) = predict_json(tmp.path(), &["--theirs", "no-such-branch"]);

    assert!(!success);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "rev_not_found");
}

#[test]
fn unrelated_histories_are_structured_no_merge_base() {
    let tmp = tempfile::tempdir().expect("temp dir");
    repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "a\nb\nc\nd\nRIGHT\n");
    git(tmp.path(), &["checkout", "-q", "--orphan", "island"]);
    git(tmp.path(), &["rm", "-rfq", "."]);
    write(tmp.path(), "other.txt", "island\n");
    git(tmp.path(), &["add", "."]);
    git(tmp.path(), &["commit", "-q", "-m", "island root"]);
    git(tmp.path(), &["checkout", "-q", "left"]);

    let (json, success) = predict_json(tmp.path(), &["--theirs", "island"]);

    assert!(!success);
    assert_eq!(json["error"]["code"], "no_merge_base");
}

#[test]
fn missing_theirs_uses_the_json_parse_error_envelope() {
    let tmp = tempfile::tempdir().expect("temp dir");
    repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "a\nb\nc\nd\nRIGHT\n");

    let output = super_git(tmp.path())
        .args(["predict", "merge"])
        .output()
        .expect("run predict merge");

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "invalid_arguments");
}

#[test]
fn human_output_summarizes_prediction() {
    let tmp = tempfile::tempdir().expect("temp dir");
    repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "RIGHT\nb\nc\nd\ne\n");

    let output = super_git(tmp.path())
        .args(["--human", "predict", "merge", "--theirs", "right"])
        .output()
        .expect("run predict merge --human");

    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("utf8 human output");
    assert!(text.contains("conflicted"));
    assert!(text.contains("f.txt (stages 1,2,3)"));
    assert!(text.contains("Limitations:"));
}
