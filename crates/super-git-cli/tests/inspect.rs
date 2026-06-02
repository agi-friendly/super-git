//! `super-git inspect`의 출력 계약 통합 테스트.
//! 실제 git 저장소를 임시로 만들고 빌드된 바이너리를 실행해 JSON envelope를 검증한다.

use std::path::Path;
use std::process::{Command, Output};

/// 빌드된 super-git 바이너리를 주어진 작업 디렉토리에서 실행할 Command를 만든다.
fn super_git(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_super-git"));
    cmd.current_dir(dir);
    cmd
}

/// 테스트용 git 실행. 전역/시스템 설정과 사용자 identity 영향을 받지 않도록 격리한다.
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

fn inspect_json(dir: &Path) -> serde_json::Value {
    let output = super_git(dir).arg("inspect").output().expect("run inspect");
    assert!(
        output.status.success(),
        "inspect failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse inspect json")
}

#[test]
fn inspect_clean_repo_reports_branch_and_no_operation() {
    let tmp = tempfile::tempdir().expect("temp dir");
    init_repo_with_commit(tmp.path());

    let json = inspect_json(tmp.path());

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["operation"], "none");
    assert_eq!(json["data"]["head"]["branch"], "main");
    assert_eq!(json["data"]["head"]["detached"], false);
    assert!(json["data"]["head"]["commit"].is_string());
}

#[test]
fn inspect_detached_head() {
    let tmp = tempfile::tempdir().expect("temp dir");
    init_repo_with_commit(tmp.path());
    git(tmp.path(), &["checkout", "-q", "--detach", "HEAD"]);

    let json = inspect_json(tmp.path());

    assert_eq!(json["data"]["head"]["detached"], true);
    assert_eq!(json["data"]["head"]["branch"], serde_json::Value::Null);
    assert!(json["data"]["head"]["commit"].is_string());
}

#[test]
fn inspect_reports_merging_during_conflict() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    // 같은 줄을 서로 다르게 바꾼 두 브랜치로 머지 충돌을 유발한다.
    git(dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("file.txt"), "feature\n").expect("write");
    git(dir, &["commit", "-q", "-am", "feature change"]);
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("file.txt"), "main\n").expect("write");
    git(dir, &["commit", "-q", "-am", "main change"]);

    // 충돌하는 merge는 exit 1로 끝나지만 MERGE_HEAD를 남긴다.
    let merge = run_git(dir, &["merge", "feature"]);
    assert!(!merge.status.success(), "merge should have conflicted");

    let json = inspect_json(dir);
    assert_eq!(json["data"]["operation"], "merging");
}

#[test]
fn inspect_reports_cherry_picking_from_sequencer_after_manual_commit() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    // cherry-pick 대상 두 커밋을 만든다.
    git(dir, &["checkout", "-q", "-b", "src"]);
    std::fs::write(dir.join("file.txt"), "hello\nA\n").expect("write");
    git(dir, &["commit", "-q", "-am", "A"]);
    std::fs::write(dir.join("file.txt"), "hello\nA\nB\n").expect("write");
    git(dir, &["commit", "-q", "-am", "B"]);
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("file.txt"), "conflict\n").expect("write");
    git(dir, &["commit", "-q", "-am", "conflict"]);

    // multi-commit cherry-pick → 충돌.
    let pick = run_git(dir, &["cherry-pick", "src~1", "src"]);
    assert!(!pick.status.success(), "cherry-pick should have conflicted");

    // --continue 대신 직접 commit하면 CHERRY_PICK_HEAD는 사라지고 sequencer/todo만 남는다.
    std::fs::write(dir.join("file.txt"), "resolved\n").expect("write");
    git(dir, &["add", "file.txt"]);
    git(dir, &["commit", "-q", "-m", "resolved"]);

    let json = inspect_json(dir);
    assert_eq!(json["data"]["operation"], "cherry-picking");
}

#[test]
fn inspect_reports_applying_during_am_conflict() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    // feature 커밋의 패치를 만들어 둔다.
    git(dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("file.txt"), "feature\n").expect("write");
    git(dir, &["commit", "-q", "-am", "feature"]);
    let patch = run_git(dir, &["format-patch", "-1", "--stdout", "HEAD"]);
    assert!(patch.status.success(), "format-patch failed");
    std::fs::write(dir.join("change.patch"), &patch.stdout).expect("write patch");

    // 같은 줄을 다르게 바꾼 main에 am을 적용해 충돌을 유발한다.
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("file.txt"), "other\n").expect("write");
    git(dir, &["commit", "-q", "-am", "other"]);

    let am = run_git(dir, &["am", "change.patch"]);
    assert!(!am.status.success(), "am should have conflicted");

    let json = inspect_json(dir);
    assert_eq!(json["data"]["operation"], "applying");
}

#[test]
fn inspect_normalizes_repository_to_worktree_root() {
    let tmp = tempfile::tempdir().expect("temp dir");
    init_repo_with_commit(tmp.path());
    let sub = tmp.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir sub");

    // 하위 디렉토리에서 실행해도 repository는 워크트리 root(절대경로)여야 한다.
    let output = super_git(&sub)
        .arg("inspect")
        .output()
        .expect("run inspect");
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");

    let repo = json["data"]["repository"]
        .as_str()
        .expect("repository is a string");
    assert!(
        Path::new(repo).is_absolute(),
        "repository should be absolute"
    );

    // symlink 차이(macOS /var -> /private/var)를 없애기 위해 양쪽 모두 canonicalize 후 비교한다.
    let repo_canon = std::fs::canonicalize(repo).expect("canonicalize repo");
    let root_canon = std::fs::canonicalize(tmp.path()).expect("canonicalize root");
    assert_eq!(repo_canon, root_canon);
}

#[test]
fn inspect_non_repo_fails_with_json_envelope() {
    let tmp = tempfile::tempdir().expect("temp dir");
    // git init을 하지 않아 git 저장소가 아니다.

    let output = super_git(tmp.path())
        .arg("inspect")
        .output()
        .expect("run inspect");
    assert!(!output.status.success(), "inspect on non-repo should fail");

    // 실패해도 JSON envelope 계약을 지켜야 한다: stdout에 { ok: false, error }, exit 1.
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("error envelope on stdout");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["message"].is_string());
}
