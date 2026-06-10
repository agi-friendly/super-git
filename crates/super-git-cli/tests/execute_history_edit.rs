//! `super-git execute` integration tests for `history_edit` plans (C8-C).
//! Execute must rebuild commits with plumbing, move the branch ref by
//! compare-and-swap, preserve the final tree and author identity, and never
//! touch the working tree.

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
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
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

/// Author is pinned per commit so author preservation can be asserted against a
/// committer that differs from the author.
fn commit_file(repo: &Path, file: &str, content: &str, message: &str) {
    std::fs::write(repo.join(file), content).expect("write file");
    git(repo, &["add", file]);
    Command::new("git")
        .current_dir(repo)
        .args(["commit", "-q", "-m", message])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Jane Author")
        .env("GIT_AUTHOR_EMAIL", "jane@example.com")
        .env("GIT_AUTHOR_DATE", "2026-06-09T10:00:00+09:00")
        .env("GIT_COMMITTER_NAME", "committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.com")
        .output()
        .expect("commit");
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

fn execute_plan_from_stdin(dir: &Path, plan: &serde_json::Value) -> Output {
    let mut command = super_git(dir);
    command
        .args(["execute", "--plan", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn execute");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(
            serde_json::to_string(plan)
                .expect("serialize plan")
                .as_bytes(),
        )
        .expect("write plan");
    child.wait_with_output().expect("wait execute")
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

fn subjects(repo: &Path, range: &str) -> Vec<String> {
    git_stdout(repo, &["log", "--reverse", "--format=%s", range])
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn reword_and_fixup_rewrites_history_preserving_tree_and_author() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let head_tree_before = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"fix(login): validate email"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    let json = json_output(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["schema_version"], "super-git.execute.v0.2");
    assert_eq!(data["action"], "history_edit");
    assert_eq!(data["executed"], true);
    assert_eq!(data["undo_token"]["kind"], "restore_branch_tip_snapshot");

    // Two commits remain and the messages reflect reword + fixup.
    let after = subjects(&repo, "main..HEAD");
    assert_eq!(
        after,
        vec![
            "feat(login): add form".to_string(),
            "fix(login): validate email".to_string()
        ]
    );
    // The tree at the tip is byte-identical: history edit preserves content.
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]),
        head_tree_before
    );
    // Author identity is preserved even though the committer differs.
    let authors = git_stdout(&repo, &["log", "--format=%an <%ae>", "main..HEAD"]);
    assert!(authors
        .lines()
        .all(|line| line == "Jane Author <jane@example.com>"));
    assert!(git_stdout(&repo, &["log", "-1", "--format=%cn"]) != "Jane Author");
    // HEAD stays attached to the branch; working tree stays clean.
    assert_eq!(
        git_stdout(&repo, &["symbolic-ref", "HEAD"]),
        "refs/heads/feature/login"
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain=v1"]), "");
}

#[test]
fn unchanged_prefix_commit_keeps_its_object_id() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    // First commit is a plain pick, so it must keep its original id; only the
    // reworded commit and everything after it gets new ids.
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"reworded b"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    json_output(execute_plan_from_stdin(&repo, &plan));

    let new_oids: Vec<String> = git_stdout(&repo, &["rev-list", "--reverse", "main..HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(new_oids.len(), 3);
    assert_eq!(new_oids[0], oids[0], "unchanged prefix keeps its object id");
    assert_ne!(new_oids[1], oids[1], "reworded commit gets a new id");
    assert_ne!(new_oids[2], oids[2], "commit after the edit is rebuilt too");
}

#[test]
fn squash_folds_two_commits_into_one_with_explicit_message() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let head_tree_before = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
    // pick a, pick b, squash c into b: c folds into the second commit, leaving two.
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"squash","message":"combined b and c"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    json_output(execute_plan_from_stdin(&repo, &plan));

    let after = subjects(&repo, "main..HEAD");
    assert_eq!(
        after,
        vec![
            "feat(login): add form".to_string(),
            "combined b and c".to_string()
        ]
    );
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]),
        head_tree_before
    );
    // All three files still exist: squash preserves content, only history shape changed.
    for file in ["a.txt", "b.txt", "c.txt"] {
        assert!(repo.join(file).exists(), "{file} should still exist");
    }
}

#[test]
fn stale_plan_after_new_commit_is_rejected_without_moving_ref() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    // Advance the branch after preview: the plan is now stale.
    commit_file(&repo, "d.txt", "d\n", "later commit");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], false);
    // The reconstructed range no longer matches, so fresh binding rejects it.
    assert!(
        cause_contains(&json, "plan_id") || cause_contains(&json, "fresh_execution_status"),
        "stale plan must be rejected: {json}"
    );
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD"]),
        tip_before,
        "stale execute must not move the branch ref"
    );
}

#[test]
fn tampered_plan_id_is_rejected_without_writing() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let mut plan = preview_plan(&repo, "main", &instructions_doc(&items));
    plan["data"]["plan_id"] = serde_json::json!("sha256:tampered");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], false);
    assert!(cause_contains(&json, "plan_id"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn published_plan_is_rejected_pending_confirmation_support() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    git(
        &repo,
        &["update-ref", "refs/remotes/origin/feature/login", &oids[2]],
    );
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));
    assert_eq!(plan["data"]["execution"]["status"], "preview_only");
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], false);
    assert!(cause_contains(&json, "confirmation_required"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn survey_plan_cannot_be_executed() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, _) = feature_repo(tmp.path());
    let plan = json_output(
        super_git(&repo)
            .args(["preview", "history-edit", "--base", "main"])
            .output()
            .expect("survey"),
    );
    let tip_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let json = error_json(execute_plan_from_stdin(&repo, &plan));

    assert_eq!(json["ok"], false);
    assert!(cause_contains(&json, "not_executable"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), tip_before);
}

#[test]
fn tampered_advisory_fields_are_ignored_in_favor_of_authentic_repo_values() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"fix: real"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let mut plan = preview_plan(&repo, "main", &instructions_doc(&items));
    // Author and the picked commit's original message are excluded from plan_id
    // (derivable from the bound object id), so a tampered plan keeps a valid id.
    // Execute must ignore these forged copies and use the authentic repo values.
    plan["data"]["range"]["commits"][0]["author_email"] = serde_json::json!("attacker@evil.com");
    plan["data"]["range"]["commits"][0]["author_name"] = serde_json::json!("Attacker");
    plan["data"]["range"]["commits"][0]["message"] = serde_json::json!("forged subject\n");

    let json = json_output(execute_plan_from_stdin(&repo, &plan));
    assert_eq!(
        json["ok"], true,
        "tampered advisory fields must still validate"
    );

    // The picked (first) commit keeps its real author and real message.
    let first_subject = git_stdout(&repo, &["log", "--reverse", "--format=%s", "main..HEAD"])
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        first_subject, "feat(login): add form",
        "forged pick message must be ignored"
    );
    let authors = git_stdout(&repo, &["log", "--format=%ae", "main..HEAD"]);
    assert!(
        authors.lines().all(|line| line == "jane@example.com"),
        "forged author must be ignored, got: {authors}"
    );
}

#[test]
fn execute_writes_completed_execution_record_with_undo_token() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let plan = preview_plan(&repo, "main", &instructions_doc(&items));

    json_output(execute_plan_from_stdin(&repo, &plan));

    let execution_dir = repo.join(".git/super-git/executions");
    let records: Vec<_> = std::fs::read_dir(&execution_dir)
        .expect("execution dir")
        .collect::<Result<_, _>>()
        .expect("read records");
    assert_eq!(records.len(), 1, "execute should write one record");
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(records[0].path()).expect("read record"))
            .expect("record json");
    assert_eq!(record["status"], "completed");
    assert_eq!(record["action"], "history_edit");
    assert_eq!(record["branch_ref"], "refs/heads/feature/login");
    assert_eq!(record["undo_token"]["kind"], "restore_branch_tip_snapshot");
    assert_eq!(record["commits_before"], 3);
    assert_eq!(record["commits_after"], 2);
}
