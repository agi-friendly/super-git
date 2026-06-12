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

/// bare origin을 만들고 clone한 뒤 첫 커밋을 push해 upstream을 설정한다. work 경로를 반환한다.
fn clone_repo_with_upstream(parent: &Path) -> std::path::PathBuf {
    let origin = parent.join("origin.git");
    let work = parent.join("work");
    git(parent, &["init", "-q", "--bare", origin.to_str().unwrap()]);
    git(
        parent,
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    std::fs::write(work.join("file.txt"), "hello\n").expect("write file");
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "init"]);
    git(&work, &["push", "-q", "-u", "origin", "HEAD"]);
    work
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

/// inspect 출력의 next bucket에서 kind 목록을 뽑는다.
fn next_kinds(json: &serde_json::Value, bucket: &str) -> Vec<String> {
    json["data"]["next"][bucket]
        .as_array()
        .unwrap_or_else(|| panic!("next.{bucket} array"))
        .iter()
        .map(|a| a["kind"].as_str().expect("kind").to_string())
        .collect()
}

fn next_action<'a>(json: &'a serde_json::Value, bucket: &str, kind: &str) -> &'a serde_json::Value {
    json["data"]["next"][bucket]
        .as_array()
        .unwrap_or_else(|| panic!("next.{bucket} array"))
        .iter()
        .find(|action| action["kind"] == kind)
        .unwrap_or_else(|| panic!("next.{bucket} missing action kind {kind}"))
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
    assert_eq!(json["data"]["working_tree"]["clean"], true);
    assert!(json["data"]["warnings"]
        .as_array()
        .expect("warnings array")
        .is_empty());
    assert_eq!(json["data"]["summary"]["state"], "ready");
    assert!(json["data"]["summary"]["codes"]
        .as_array()
        .expect("summary codes")
        .iter()
        .any(|code| code == "working_tree_clean"));
    assert_eq!(json["data"]["risk_hint"]["level"], "low");
    assert!(json["data"]["risk_hint"]["factors"]
        .as_array()
        .expect("risk factors")
        .is_empty());
    assert_eq!(json["data"]["schema_version"], "super-git.inspect.v0.3");
    assert_eq!(json["data"]["summary"]["state_scope"], "repository_posture");
    assert_eq!(
        json["data"]["summary"]["execution_permission"],
        "not_granted_by_inspect"
    );
    assert_eq!(
        json["data"]["summary"]["message"],
        "Repository is ready for preview selection."
    );
    assert_eq!(json["data"]["risk_hint"]["scope"], "current_state_only");
    assert_eq!(json["data"]["next"]["scope"], "inspect_state_only");
    assert_eq!(
        json["data"]["next"]["execution_contract"],
        "preview_required"
    );
    assert_eq!(
        json["data"]["next"]["allowed_semantics"],
        "preview_candidate"
    );
    assert_eq!(json["data"]["next"]["blocked_semantics"], "state_guardrail");
    assert_eq!(
        json["data"]["next"]["needs_human_review_scope"],
        "evaluated_inspect_actions_only"
    );
    assert_eq!(json["data"]["next"]["raw_git_allowed"], false);
    let evaluated_actions: Vec<_> = json["data"]["next"]["evaluated_actions"]
        .as_array()
        .expect("evaluated_actions array")
        .iter()
        .map(|action| action.as_str().expect("evaluated action string"))
        .collect();
    assert_eq!(
        evaluated_actions,
        vec![
            "stage_changes",
            "commit",
            "push",
            "pull",
            "integrate_diverged",
            "resolve_conflicts",
            "continue_operation",
            "merge_continue",
            "merge_abort",
            "rebase_continue",
            "rebase_skip",
            "rebase_abort",
            "am_continue",
            "am_skip",
            "am_abort",
            "cherry_pick_continue",
            "cherry_pick_skip",
            "cherry_pick_abort",
            "revert_continue",
            "revert_skip",
            "revert_abort",
            "bisect_reset",
            "worktree_create",
            "history_edit",
        ]
    );
    assert!(!json["data"]
        .as_object()
        .expect("inspect data object")
        .contains_key("allowed_next"));
    // clean + upstream 없음 → 일반 흐름 제안은 없고 preview 진입점 후보만 남는다.
    assert_eq!(
        next_kinds(&json, "allowed"),
        vec!["worktree_create", "history_edit"]
    );
    assert!(next_kinds(&json, "blocked").is_empty());
    assert!(next_kinds(&json, "needs_human_review").is_empty());
    // reference_command는 placeholder(<ref>/<base>)를 포함한 preview 진입점이다.
    assert_eq!(
        next_action(&json, "allowed", "worktree_create")["reference_command"],
        serde_json::json!(["super-git", "preview", "worktree-create", "--ref", "<ref>"])
    );
    assert_eq!(
        next_action(&json, "allowed", "history_edit")["reference_command"],
        serde_json::json!(["super-git", "preview", "history-edit", "--base", "<base>"])
    );
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
    // detached여도 worktree_create는 가능하지만 history_edit는 attached HEAD가 필요하다.
    assert!(next_kinds(&json, "allowed")
        .iter()
        .any(|k| k == "worktree_create"));
    assert!(!next_kinds(&json, "allowed")
        .iter()
        .any(|k| k == "history_edit"));
    assert!(next_kinds(&json, "blocked")
        .iter()
        .any(|k| k == "history_edit"));
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
    // 충돌 파일이 working_tree.conflicts에 잡힌다.
    let wt = &json["data"]["working_tree"];
    assert_eq!(wt["conflict_count"], 1);
    assert_eq!(wt["conflicts"][0], "file.txt");
    assert_eq!(wt["clean"], false);
    assert_eq!(json["data"]["summary"]["state"], "blocked");
    assert_eq!(json["data"]["risk_hint"]["level"], "high");
    assert!(json["data"]["risk_hint"]["factors"]
        .as_array()
        .expect("risk factors")
        .iter()
        .any(|factor| factor["code"] == "conflicts_present"));

    let allowed = next_kinds(&json, "allowed");
    assert!(allowed.iter().any(|k| k == "resolve_conflicts"));
    assert!(allowed.iter().any(|k| k == "merge_abort"));
    let blocked = next_kinds(&json, "blocked");
    assert!(blocked.iter().any(|k| k == "continue_operation"));
    // 진행 중 작업/충돌에서는 resolve 흐름이 우선이다:
    // history_edit는 blocked로 사유를 보여주고 worktree_create는 노출하지 않는다.
    assert!(blocked.iter().any(|k| k == "history_edit"));
    assert!(!allowed.iter().any(|k| k == "worktree_create"));
    assert!(!blocked.iter().any(|k| k == "worktree_create"));
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
    assert_eq!(
        next_action(&json, "allowed", "cherry_pick_abort")["reference_command"],
        serde_json::json!(["git", "cherry-pick", "--abort"])
    );
    assert!(!next_action(&json, "allowed", "cherry_pick_abort")
        .as_object()
        .expect("action object")
        .contains_key("command"));
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

#[test]
fn inspect_reports_no_upstream_without_remote() {
    let tmp = tempfile::tempdir().expect("temp dir");
    init_repo_with_commit(tmp.path());

    let json = inspect_json(tmp.path());
    assert_eq!(json["data"]["upstream"], serde_json::Value::Null);
}

#[test]
fn inspect_reports_upstream_ahead() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let work = clone_repo_with_upstream(tmp.path());

    // 로컬에만 커밋을 추가하면 upstream 대비 ahead 1, behind 0이 된다.
    std::fs::write(work.join("file.txt"), "more\n").expect("write");
    git(&work, &["commit", "-q", "-am", "local change"]);

    let json = inspect_json(&work);
    let upstream = &json["data"]["upstream"];
    assert!(
        upstream["name"]
            .as_str()
            .expect("upstream name")
            .starts_with("origin/"),
        "upstream name should be origin/*, got {:?}",
        upstream["name"]
    );
    assert_eq!(upstream["ahead"], 1);
    assert_eq!(upstream["behind"], 0);
    assert_eq!(upstream["comparison_basis"], "local_tracking_ref");
    assert_eq!(upstream["comparison_status"], "ok");
    let warnings = json["data"]["warnings"].as_array().expect("warnings array");
    assert!(warnings
        .iter()
        .any(|w| w["code"] == "upstream_freshness_unknown"));
    assert!(next_kinds(&json, "allowed").iter().any(|k| k == "push"));
}

#[test]
fn inspect_marks_failed_upstream_comparison() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    // upstream 이름은 해석되지만 rev-list 비교는 실패하는 ref를 만든다.
    let origin = tmp.path().join("origin.git");
    git(dir, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(dir, &["config", "branch.main.remote", "origin"]);
    git(dir, &["config", "branch.main.merge", "refs/heads/main"]);
    let remote_ref = dir.join(".git/refs/remotes/origin");
    std::fs::create_dir_all(&remote_ref).expect("mkdir remote ref dir");
    std::fs::write(
        remote_ref.join("main"),
        "0000000000000000000000000000000000000001\n",
    )
    .expect("write invalid remote ref");

    let json = inspect_json(dir);
    let upstream = &json["data"]["upstream"];
    assert_eq!(upstream["name"], "origin/main");
    assert_eq!(upstream["ahead"], 0);
    assert_eq!(upstream["behind"], 0);
    assert_eq!(upstream["comparison_basis"], "local_tracking_ref");
    assert_eq!(upstream["comparison_status"], "failed");
    let summary_codes = json["data"]["summary"]["codes"]
        .as_array()
        .expect("summary codes");
    assert!(summary_codes
        .iter()
        .any(|code| code == "upstream_comparison_failed"));
    assert!(!summary_codes.iter().any(|code| code == "upstream_synced"));
    let warnings = json["data"]["warnings"].as_array().expect("warnings array");
    assert!(warnings
        .iter()
        .any(|w| w["code"] == "upstream_comparison_failed"));
}

#[test]
fn inspect_reports_working_tree_changes() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    // staged 1, unstaged 1, untracked 1 상태를 만든다.
    std::fs::write(dir.join("staged.txt"), "s\n").expect("write");
    git(dir, &["add", "staged.txt"]);
    std::fs::write(dir.join("file.txt"), "modified\n").expect("write"); // 추적 파일 수정 → unstaged
    std::fs::write(dir.join("untracked.txt"), "u\n").expect("write");

    let json = inspect_json(dir);
    let wt = &json["data"]["working_tree"];
    assert_eq!(wt["clean"], false);
    assert_eq!(wt["staged"], 1);
    assert_eq!(wt["unstaged"], 1);
    assert_eq!(wt["untracked"], 1);
    assert_eq!(wt["conflict_count"], 0);
    assert_eq!(json["data"]["summary"]["state"], "dirty");
    assert_eq!(json["data"]["risk_hint"]["level"], "medium");
    assert!(json["data"]["risk_hint"]["factors"]
        .as_array()
        .expect("risk factors")
        .iter()
        .any(|factor| factor["code"] == "working_tree_dirty"));
    // staged가 있으니 commit, unstaged/untracked가 있으니 stage_changes 둘 다 제안된다.
    let ks = next_kinds(&json, "allowed");
    assert!(ks.iter().any(|k| k == "commit"));
    assert!(ks.iter().any(|k| k == "stage_changes"));
}

#[test]
fn inspect_reports_upstream_behind() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let work = clone_repo_with_upstream(tmp.path());
    let origin = tmp.path().join("origin.git");

    // 다른 clone에서 커밋을 push해 origin을 앞서게 만든다.
    let work2 = tmp.path().join("work2");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            work2.to_str().unwrap(),
        ],
    );
    std::fs::write(work2.join("file.txt"), "remote\n").expect("write");
    git(&work2, &["commit", "-q", "-am", "remote change"]);
    git(&work2, &["push", "-q"]);

    // work는 fetch만 하면 upstream보다 뒤처진다.
    git(&work, &["fetch", "-q"]);

    let json = inspect_json(&work);
    let upstream = &json["data"]["upstream"];
    assert_eq!(upstream["ahead"], 0);
    assert_eq!(upstream["behind"], 1);
}

#[test]
fn inspect_blocks_pull_when_dirty_and_behind() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let work = clone_repo_with_upstream(tmp.path());
    let origin = tmp.path().join("origin.git");

    let work2 = tmp.path().join("work2");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            work2.to_str().unwrap(),
        ],
    );
    std::fs::write(work2.join("file.txt"), "remote\n").expect("write");
    git(&work2, &["commit", "-q", "-am", "remote change"]);
    git(&work2, &["push", "-q"]);
    git(&work, &["fetch", "-q"]);

    std::fs::write(work.join("local.txt"), "local dirty\n").expect("write dirty file");

    let json = inspect_json(&work);
    let allowed = next_kinds(&json, "allowed");
    assert!(allowed.iter().any(|k| k == "stage_changes"));
    assert!(!allowed.iter().any(|k| k == "pull"));
    let blocked = next_kinds(&json, "blocked");
    assert!(blocked.iter().any(|k| k == "pull"));
}

#[test]
fn inspect_reports_upstream_diverged() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let work = clone_repo_with_upstream(tmp.path());
    let origin = tmp.path().join("origin.git");

    // 다른 clone이 origin을 앞서게 한다.
    let work2 = tmp.path().join("work2");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            work2.to_str().unwrap(),
        ],
    );
    std::fs::write(work2.join("file.txt"), "remote\n").expect("write");
    git(&work2, &["commit", "-q", "-am", "remote change"]);
    git(&work2, &["push", "-q"]);

    // work는 로컬 커밋(ahead) 후 fetch(behind)로 갈라진다.
    std::fs::write(work.join("local.txt"), "local\n").expect("write");
    git(&work, &["add", "local.txt"]);
    git(&work, &["commit", "-q", "-m", "local change"]);
    git(&work, &["fetch", "-q"]);

    let json = inspect_json(&work);
    let upstream = &json["data"]["upstream"];
    assert_eq!(upstream["ahead"], 1);
    assert_eq!(upstream["behind"], 1);
}

#[test]
fn inspect_reports_merge_continue_when_conflicts_resolved() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    git(dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("file.txt"), "feature\n").expect("write");
    git(dir, &["commit", "-q", "-am", "feature change"]);
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("file.txt"), "main\n").expect("write");
    git(dir, &["commit", "-q", "-am", "main change"]);

    let merge = run_git(dir, &["merge", "feature"]);
    assert!(!merge.status.success(), "merge should have conflicted");

    // 충돌 해결 후 add까지(commit은 하지 않음) → 여전히 merging, 충돌 0.
    std::fs::write(dir.join("file.txt"), "resolved\n").expect("write");
    git(dir, &["add", "file.txt"]);

    let json = inspect_json(dir);
    assert_eq!(json["data"]["operation"], "merging");
    assert_eq!(json["data"]["working_tree"]["conflict_count"], 0);

    let ks = next_kinds(&json, "allowed");
    assert!(ks.iter().any(|k| k == "merge_continue"));
    assert!(!ks.iter().any(|k| k == "resolve_conflicts"));
}

#[test]
fn inspect_reports_rebase_conflict_without_continue() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    git(dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("file.txt"), "feature\n").expect("write");
    git(dir, &["commit", "-q", "-am", "feature change"]);
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("file.txt"), "main\n").expect("write");
    git(dir, &["commit", "-q", "-am", "main change"]);

    // feature를 main 위로 rebase하면 file.txt에서 충돌한다.
    git(dir, &["checkout", "-q", "feature"]);
    let rebase = run_git(dir, &["rebase", "main"]);
    assert!(!rebase.status.success(), "rebase should have conflicted");

    let json = inspect_json(dir);
    assert_eq!(json["data"]["operation"], "rebasing");

    let ks = next_kinds(&json, "allowed");
    assert!(ks.iter().any(|k| k == "resolve_conflicts"));
    assert!(ks.iter().any(|k| k == "rebase_abort"));
    // 충돌 해결 전에는 continue를 제안하지 않는다.
    assert!(!ks.iter().any(|k| k == "rebase_continue"));
}

#[test]
fn inspect_main_worktree_context() {
    let tmp = tempfile::tempdir().expect("temp dir");
    init_repo_with_commit(tmp.path());

    let json = inspect_json(tmp.path());
    let wc = &json["data"]["worktree_context"];
    assert_eq!(wc["kind"], "main");
    assert_eq!(wc["family_count"], 1);
    assert_eq!(wc["linked_count"], 0);
}

#[test]
fn inspect_linked_worktree_context() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    // linked worktree를 추가하고 그 안에서 inspect한다.
    let linked = dir.join("linked");
    git(dir, &["worktree", "add", "-q", linked.to_str().unwrap()]);

    let json = inspect_json(&linked);
    let wc = &json["data"]["worktree_context"];
    assert_eq!(wc["kind"], "linked");
    assert_eq!(wc["family_count"], 2);
    assert_eq!(wc["linked_count"], 1);

    // main은 원본 repo를 가리킨다(symlink 차이 제거 위해 canonicalize 후 비교).
    let main_canon =
        std::fs::canonicalize(wc["main"].as_str().expect("main path")).expect("canon main");
    let dir_canon = std::fs::canonicalize(dir).expect("canon dir");
    assert_eq!(main_canon, dir_canon);
}

#[test]
fn inspect_bare_primary_worktree_has_null_main() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();

    // 일반 repo에서 커밋을 만들고 bare로 clone한 뒤, bare에 linked worktree를 단다.
    let src = dir.join("src");
    std::fs::create_dir(&src).expect("mkdir src");
    init_repo_with_commit(&src);
    let bare = dir.join("bare.git");
    git(
        dir,
        &[
            "clone",
            "--bare",
            "-q",
            src.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let wt = dir.join("wt");
    git(
        &bare,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "main"],
    );

    // bare-primary family의 linked worktree에서 inspect.
    let json = inspect_json(&wt);
    let wc = &json["data"]["worktree_context"];
    assert_eq!(wc["kind"], "linked");
    // bare-primary family에는 main worktree가 없으므로 null이어야 한다.
    assert_eq!(wc["main"], serde_json::Value::Null);
}

#[test]
fn inspect_ignores_ambient_git_object_directory() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    // A bogus object directory in the ambient env would hide every object, so a
    // tool that inherited it could not resolve HEAD. The command wrapper scrubs
    // it, keeping inspect bound to the repository selected by `git -C`.
    let bogus = tmp.path().join("nonexistent-objects");
    let output = super_git(dir)
        .env("GIT_OBJECT_DIRECTORY", &bogus)
        .arg("inspect")
        .output()
        .expect("run inspect");

    assert!(
        output.status.success(),
        "inspect must ignore ambient GIT_OBJECT_DIRECTORY: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["data"]["head"]["branch"], "main");
    assert_eq!(json["data"]["head"]["detached"], false);
}

#[cfg(unix)]
#[test]
fn inspect_ignores_ambient_git_config_fsmonitor_injection() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);

    // Ambient GIT_CONFIG_COUNT/KEY/VALUE inject arbitrary config, and
    // core.fsmonitor is run as a command on read operations (GIT_OPTIONAL_LOCKS=0
    // does not suppress it). The driver below touches a sentinel; the scrub
    // clears GIT_CONFIG_COUNT so the injection never runs.
    let sentinel = tmp.path().join("pwned");
    let driver = tmp.path().join("fsmonitor.sh");
    std::fs::write(
        &driver,
        format!("#!/bin/sh\ntouch '{}'\n", sentinel.display()),
    )
    .expect("write driver");
    let mut perms = std::fs::metadata(&driver)
        .expect("driver meta")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&driver, perms).expect("chmod driver");

    let output = super_git(dir)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.fsmonitor")
        .env("GIT_CONFIG_VALUE_0", driver.to_str().expect("driver path"))
        .arg("inspect")
        .output()
        .expect("run inspect");

    assert!(
        output.status.success(),
        "inspect failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !sentinel.exists(),
        "ambient GIT_CONFIG core.fsmonitor injection must not run a command"
    );
}

#[test]
fn inspect_reports_untracked_despite_show_untracked_files_no() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    // A repo (or user) config of status.showUntrackedFiles=no hides untracked
    // files from plain `git status`; inspect pins --untracked-files=all so it
    // still sees them and does not report a clean tree.
    git(dir, &["config", "status.showUntrackedFiles", "no"]);
    std::fs::write(dir.join("untracked.txt"), "u\n").expect("write untracked");

    let json = inspect_json(dir);

    assert_eq!(json["data"]["working_tree"]["untracked"], 1);
    assert_eq!(json["data"]["working_tree"]["clean"], false);
}

#[cfg(unix)]
#[test]
fn inspect_does_not_run_repo_local_fsmonitor() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    init_repo_with_commit(dir);
    // A hostile repo can set core.fsmonitor in its own .git/config; git runs it
    // as a command even on read-only inspection. Read commands disable fsmonitor
    // via -c, so the driver must not run.
    let sentinel = tmp.path().join("pwned");
    let driver = tmp.path().join("fsmonitor.sh");
    std::fs::write(
        &driver,
        format!("#!/bin/sh\ntouch '{}'\n", sentinel.display()),
    )
    .expect("write driver");
    let mut perms = std::fs::metadata(&driver)
        .expect("driver meta")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&driver, perms).expect("chmod driver");
    git(
        dir,
        &[
            "config",
            "core.fsmonitor",
            driver.to_str().expect("driver path"),
        ],
    );

    let output = super_git(dir).arg("inspect").output().expect("run inspect");

    assert!(
        output.status.success(),
        "inspect failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !sentinel.exists(),
        "a repo-local core.fsmonitor must not run on read-only inspect"
    );
}

#[test]
fn inspect_reports_unborn_head_on_fresh_init() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    // A fresh `git init` with zero commits: the state every bootstrap agent
    // starts from. inspect must succeed and say so, not error out.
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("new.txt"), "x\n").expect("write untracked");

    let json = inspect_json(dir);

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["head"]["commit"], serde_json::Value::Null);
    assert_eq!(json["data"]["working_tree"]["untracked"], 1);
    assert_eq!(json["data"]["working_tree"]["clean"], false);
    // unborn HEAD: 편집할 history도, worktree를 시작할 ref도 아직 없다.
    let blocked = next_kinds(&json, "blocked");
    assert!(blocked.iter().any(|k| k == "history_edit"));
    assert!(blocked.iter().any(|k| k == "worktree_create"));
    let allowed = next_kinds(&json, "allowed");
    assert!(!allowed.iter().any(|k| k == "history_edit"));
    assert!(!allowed.iter().any(|k| k == "worktree_create"));
}

#[test]
fn preview_history_edit_on_unborn_head_fails_with_clear_code() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);

    let output = super_git(dir)
        .args(["preview", "history-edit", "--base", "main"])
        .output()
        .expect("run preview history-edit");

    assert!(!output.status.success(), "unborn HEAD cannot be edited");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "head_unborn");
}
