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
    let output = execute_expect_error(dir, plan);
    assert!(
        output.status.success(),
        "execute failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse execute json")
}

/// Runs execute without asserting success, so a test can inspect a rejection.
fn execute_expect_error(dir: &Path, plan: &serde_json::Value) -> Output {
    let mut command = super_git(dir);
    command.args(["execute", "--plan", "-"]);
    pipe_stdin(command, &serde_json::to_string(plan).expect("plan json"))
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

    // A successful undo consumes the execution record (so the plan can be
    // executed again); a replayed token therefore fails on the missing
    // provenance anchor rather than moving the branch a second time.
    let json = error_json(undo(&repo, &result));
    assert!(cause_contains(&json, "execution_record_missing"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

/// Record consumption is part of the undo success contract: if the ref and
/// working tree are restored but the record cannot be removed, undo must report
/// a structured partial failure instead of `{ ok: true }`. Otherwise a fresh
/// preview of the same state reproduces the same state-based plan id, hits the
/// surviving record, and is rejected as `execution_already_attempted` while the
/// user was told the plan could be executed again.
#[cfg(unix)]
#[test]
fn undo_reports_partial_failure_when_record_consumption_fails() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let plan = preview_plan(&repo, "main", &reword_fixup_items(&oids));
    let result = execute(&repo, &plan);
    let tip_after_execute = git_stdout(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(tip_after_execute, tip_before);

    // Make the executions directory read-only so removing the record fails
    // (a containing-directory write is required to unlink an entry). This test
    // assumes a non-root runner, the standard case for `cargo test`.
    let executions = repo.join(".git/super-git/executions");
    let original = std::fs::metadata(&executions).expect("meta").permissions();
    std::fs::set_permissions(&executions, std::fs::Permissions::from_mode(0o555))
        .expect("lock dir");

    let json = error_json(undo(&repo, &result));

    // Restore permissions before any assertion can panic and leak the lock.
    std::fs::set_permissions(&executions, original).expect("restore dir");

    // The ref restore happened before the record removal, so the branch is back.
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
    assert_eq!(json["error"]["code"], "undo_partial_failure");
    let details = &json["error"]["details"];
    assert_eq!(details["status"], "failed_partial");
    assert_eq!(details["sync_completed"], true);
    assert!(details["cleanup"]["safe_next"]
        .as_str()
        .unwrap_or_default()
        .contains("execution_already_attempted"));

    // The record survived, so the same plan really is blocked from re-execute -
    // which is exactly why undo must not have claimed success.
    let records = std::fs::read_dir(&executions).expect("dir").count();
    assert_eq!(records, 1, "the record must remain when removal failed");
    let reexecute = error_json(execute_expect_error(&repo, &plan));
    assert!(cause_contains(&reexecute, "execution_already_attempted"));
}

// ---- C8-drop-D: drop undo (restore_branch_tip_and_worktree) ----

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

/// Runs execute with the plan's matching confirmation; artifacts live outside
/// the repo because drop's clean-tree gate counts untracked files.
fn execute_confirmed(repo: &Path, plan: &serde_json::Value) -> Output {
    let artifacts = repo.parent().expect("repo has a parent dir");
    let plan_file = artifacts.join("plan.json");
    let confirmation_file = artifacts.join("confirmation.json");
    std::fs::write(&plan_file, serde_json::to_vec(plan).expect("plan")).expect("write plan");
    std::fs::write(
        &confirmation_file,
        serde_json::to_vec(&confirmation_for(plan)).expect("confirmation"),
    )
    .expect("write confirmation");
    super_git(repo)
        .arg("execute")
        .arg("--plan")
        .arg(&plan_file)
        .arg("--confirmation")
        .arg(&confirmation_file)
        .output()
        .expect("run execute")
}

fn drop_middle_items(oids: &[String]) -> String {
    instructions_doc(&format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    ))
}

#[test]
fn drop_undo_restores_ref_index_and_worktree() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let plan = preview_plan(&repo, "main", &drop_middle_items(&oids));
    let result = json_output(execute_confirmed(&repo, &plan));
    assert_eq!(
        result["data"]["undo_token"]["kind"],
        "restore_branch_tip_and_worktree"
    );
    assert!(!repo.join("b.txt").exists(), "execute dropped the patch");

    let json = json_output(undo(&repo, &result));

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["undone"], true);
    // The branch, HEAD, index, and working tree are all back at the
    // pre-execute tip — the symmetric inverse of drop execute.
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
    assert_eq!(
        git_stdout(&repo, &["symbolic-ref", "HEAD"]),
        "refs/heads/feature/login"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("b.txt")).expect("read"),
        "b\n",
        "the dropped patch is back in the working tree"
    );
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        ""
    );
    let restored: Vec<String> = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        restored, oids,
        "the original commit chain is reachable again"
    );
}

#[test]
fn drop_execute_undo_re_execute_roundtrip() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &drop_middle_items(&oids));

    let result = json_output(execute_confirmed(&repo, &plan));
    json_output(undo(&repo, &result));

    // Undo consumed the execution record, so the identical plan (state-based
    // plan_id) and the identical confirmation execute again instead of dying
    // on execution_already_attempted.
    let rerun = json_output(execute_confirmed(&repo, &plan));
    assert_eq!(rerun["ok"], true);
    assert!(!repo.join("b.txt").exists());
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        ""
    );
}

#[test]
fn drop_undo_refuses_dirty_trees_and_succeeds_after_cleanup() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &drop_middle_items(&oids));
    let result = json_output(execute_confirmed(&repo, &plan));
    let tip_after_execute = git_stdout(&repo, &["rev-parse", "HEAD"]);

    // Tracked edit: refuse before any write.
    std::fs::write(repo.join("a.txt"), "local edit\n").expect("dirty");
    let json = error_json(undo(&repo, &result));
    assert!(cause_contains(&json, "working_tree_clean"), "{json}");
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_after_execute);
    assert_eq!(
        std::fs::read_to_string(repo.join("a.txt")).expect("read"),
        "local edit\n"
    );

    // Untracked counts as dirty too: the sync revives the dropped path.
    git(&repo, &["checkout", "-q", "--", "a.txt"]);
    std::fs::write(repo.join("scratch.txt"), "untracked\n").expect("untracked");
    let json = error_json(undo(&repo, &result));
    assert!(cause_contains(&json, "working_tree_clean"));

    // The refused undo consumed nothing: after cleanup the same token works.
    std::fs::remove_file(repo.join("scratch.txt")).expect("cleanup");
    json_output(undo(&repo, &result));
    assert!(repo.join("b.txt").exists());
}

/// undo 방향의 ignored 충돌 셋업: force-add된 ignored 파일을 가진 커밋을
/// drop하면 execute가 그 경로를 워킹트리에서 제거한다. 그 자리에 사용자가
/// 새 ignored 파일을 만들어 두면, undo의 pre-execute tip 동기화가 그 파일을
/// 덮어쓰는 경로가 된다 — execute 쪽 collision 게이트의 정확한 역방향.
fn ignored_revival_repo_for_undo(temp: &Path) -> (std::path::PathBuf, Vec<String>) {
    let repo = temp.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "committer"]);
    git(&repo, &["config", "user.email", "committer@example.com"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join(".gitignore"), "ignored.txt\nscratch.log\n").expect("gitignore");
    git(&repo, &["add", ".gitignore"]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    git(&repo, &["checkout", "-q", "-b", "feature/login"]);
    std::fs::write(repo.join("ignored.txt"), "tracked precious\n").expect("write");
    git(&repo, &["add", "-f", "ignored.txt"]);
    git(&repo, &["commit", "-q", "-m", "force-add ignored.txt"]);
    commit_file(&repo, "a.txt", "a\n", "unrelated");
    let oids = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    (repo, oids)
}

#[test]
fn drop_undo_refuses_ignored_collisions_then_allows_harmless_ignored_files() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = ignored_revival_repo_for_undo(tmp.path());
    // Drop the force-add commit: the new tip stops tracking ignored.txt and
    // execute removes it from the working tree.
    let items = instructions_doc(&format!(
        r#"[{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1]
    ));
    let plan = preview_plan(&repo, "main", &items);
    let result = json_output(execute_confirmed(&repo, &plan));
    assert!(!repo.join("ignored.txt").exists());
    let tip_after_execute = git_stdout(&repo, &["rev-parse", "HEAD"]);

    // The user drops a fresh ignored file where the pre-execute tip tracks
    // one. The status gate cannot see it; the collision gate must.
    std::fs::write(repo.join("ignored.txt"), "NEW LOCAL DATA\n").expect("local file");
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        ""
    );
    let json = error_json(undo(&repo, &result));
    assert!(cause_contains(&json, "ignored_path_collision"), "{json}");
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_after_execute);
    assert_eq!(
        std::fs::read_to_string(repo.join("ignored.txt")).expect("read"),
        "NEW LOCAL DATA\n",
        "the local ignored file must survive byte-identical"
    );

    // Non-colliding ignored files do not block undo and survive the sync.
    std::fs::remove_file(repo.join("ignored.txt")).expect("cleanup");
    std::fs::write(repo.join("scratch.log"), "build noise\n").expect("scratch");
    json_output(undo(&repo, &result));
    assert_eq!(
        std::fs::read_to_string(repo.join("ignored.txt")).expect("read"),
        "tracked precious\n",
        "undo revives the tracked content"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("scratch.log")).expect("read"),
        "build noise\n"
    );
}

#[test]
fn forged_token_kind_downgrade_is_rejected() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &drop_middle_items(&oids));
    let mut result = json_output(execute_confirmed(&repo, &plan));
    let tip_after_execute = git_stdout(&repo, &["rev-parse", "HEAD"]);
    // Downgrading the kind would skip the working-tree restore and leave the
    // dropped content resurrected as staged changes. The record's embedded
    // token must not match.
    result["data"]["undo_token"]["kind"] = serde_json::json!("restore_branch_tip_snapshot");

    let json = error_json(undo(&repo, &result));

    assert!(cause_contains(&json, "execution_record_token_mismatch"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_after_execute);
    assert!(!repo.join("b.txt").exists(), "nothing may move on refusal");
}

#[test]
fn undo_consumes_the_record_so_the_same_plan_re_executes() {
    // The tree-preserving family gets the same lifecycle: without record
    // consumption the state-based plan_id would lock this edit out of the
    // branch forever (a fresh preview lands on the same record path).
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let plan = preview_plan(&repo, "main", &reword_fixup_items(&oids));

    let result = execute(&repo, &plan);
    json_output(undo(&repo, &result));

    let rerun = execute(&repo, &plan);
    assert_eq!(rerun["ok"], true);
}

#[test]
fn drop_undo_respects_the_sparse_checkout_cone() {
    // Undo's reverse sync uses the same read-tree -u --reset primitive as
    // execute, so it must respect a sparse cone too: the revived in-cone file
    // comes back while tracked outside paths stay non-materialized.
    let tmp = tempfile::tempdir().expect("temp");
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    git(&repo, &["checkout", "-q", "-b", "feature/login"]);
    std::fs::create_dir_all(repo.join("inside")).expect("mkdir inside");
    std::fs::create_dir_all(repo.join("outside")).expect("mkdir outside");
    std::fs::write(repo.join("inside/keep.txt"), "keep\n").expect("write");
    std::fs::write(repo.join("outside/x.txt"), "x\n").expect("write");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "seed"]);
    commit_file(&repo, "outside/y.txt", "y\n", "outside change");
    commit_file(&repo, "inside/b.txt", "b\n", "inside b to drop");
    commit_file(&repo, "inside/c.txt", "c\n", "inside c to keep");
    let oids: Vec<String> = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    git(&repo, &["sparse-checkout", "set", "inside"]);

    let items = instructions_doc(&format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"drop"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2], oids[3]
    ));
    let plan = preview_plan(&repo, "main", &items);
    let result = json_output(execute_confirmed(&repo, &plan));
    assert!(!repo.join("inside/b.txt").exists());

    let undone = json_output(undo(&repo, &result));
    assert_eq!(undone["ok"], true);
    assert!(
        repo.join("inside/b.txt").exists(),
        "undo revives the dropped in-cone file"
    );
    assert!(
        !repo.join("outside/x.txt").exists(),
        "the cone stays respected through undo's reverse sync"
    );
    assert_eq!(
        git_stdout(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        ""
    );
}
