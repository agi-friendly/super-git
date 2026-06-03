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

/// inspect 출력의 allowed_next에서 kind 목록을 뽑는다.
fn action_kinds(json: &serde_json::Value) -> Vec<String> {
    json["data"]["allowed_next"]
        .as_array()
        .expect("allowed_next array")
        .iter()
        .map(|a| a["kind"].as_str().expect("kind").to_string())
        .collect()
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
    // clean + upstream 없음 → 제안할 행동이 없다.
    assert!(action_kinds(&json).is_empty());
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
    // 충돌 파일이 working_tree.conflicts에 잡힌다.
    let wt = &json["data"]["working_tree"];
    assert_eq!(wt["conflict_count"], 1);
    assert_eq!(wt["conflicts"][0], "file.txt");
    assert_eq!(wt["clean"], false);

    let ks = action_kinds(&json);
    assert!(ks.iter().any(|k| k == "resolve-conflicts"));
    assert!(ks.iter().any(|k| k == "merge-abort"));
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
    assert!(action_kinds(&json).iter().any(|k| k == "push"));
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
    // staged가 있으니 commit, unstaged/untracked가 있으니 stage-changes 둘 다 제안된다.
    let ks = action_kinds(&json);
    assert!(ks.iter().any(|k| k == "commit"));
    assert!(ks.iter().any(|k| k == "stage-changes"));
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

    let ks = action_kinds(&json);
    assert!(ks.iter().any(|k| k == "merge-continue"));
    assert!(!ks.iter().any(|k| k == "resolve-conflicts"));
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

    let ks = action_kinds(&json);
    assert!(ks.iter().any(|k| k == "resolve-conflicts"));
    assert!(ks.iter().any(|k| k == "rebase-abort"));
    // 충돌 해결 전에는 continue를 제안하지 않는다.
    assert!(!ks.iter().any(|k| k == "rebase-continue"));
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
