//! `super-git preview history-edit` integration tests.
//! Preview must be read-only and must emit a stable `super-git.plan.v0.5` plan.

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

fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = run_git(dir, args);
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Local config pins identity and signing so the plan stays deterministic
/// regardless of the developer's real global Git configuration.
fn init_repo(repo: &Path) {
    std::fs::create_dir_all(repo).expect("create repo");
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "user.name", "test"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
    commit_file(repo, "README.md", "hello\n", "initial");
}

fn commit_file(repo: &Path, file: &str, content: &str, message: &str) {
    std::fs::write(repo.join(file), content).expect("write file");
    git(repo, &["add", file]);
    git(repo, &["commit", "-q", "-m", message]);
}

/// A feature branch with three editable commits over `main`.
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

fn preview_survey(dir: &Path, base: &str) -> Output {
    super_git(dir)
        .args(["preview", "history-edit", "--base", base])
        .output()
        .expect("run preview history-edit survey")
}

fn preview_with_instructions(dir: &Path, base: &str, instructions: &str) -> Output {
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
    child.wait_with_output().expect("wait preview")
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

fn instructions_doc(items: &str) -> String {
    format!(
        r#"{{"schema_version":"super-git.instructions.v0.1","action":"history_edit","base":"main","items":{items}}}"#
    )
}

fn blocked_codes(data: &serde_json::Value) -> Vec<&str> {
    data["execution"]["blocked_reasons"]
        .as_array()
        .expect("blocked reasons")
        .iter()
        .map(|reason| reason["code"].as_str().expect("code"))
        .collect()
}

fn for_each_ref(repo: &Path) -> String {
    git_stdout(repo, &["for-each-ref"])
}

#[test]
fn survey_without_instructions_emits_read_only_plan() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let before_refs = for_each_ref(&repo);

    let json = json_output(preview_survey(&repo, "main"));

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["schema_version"], "super-git.plan.v0.5");
    assert!(data["plan_id"]
        .as_str()
        .expect("plan id")
        .starts_with("sha256:"));
    assert_eq!(data["action"]["kind"], "history_edit");
    assert_eq!(data["action"]["options"]["base"], "main");
    assert_eq!(data["branch"]["ref"], "refs/heads/feature/login");
    assert_eq!(data["branch"]["tip_commit"], oids[2]);
    assert_eq!(data["range"]["commit_count"], 3);
    assert_eq!(
        data["range"]["commits"][0]["message"],
        "feat(login): add form\n"
    );
    assert_eq!(data["execution"]["status"], "survey");
    assert_eq!(data["execution"]["execute_supported"], false);
    assert_eq!(data["instructions"], serde_json::Value::Null);
    assert_eq!(data["result_summary"], serde_json::Value::Null);
    assert!(data.get("confirmation").is_none());
    assert_eq!(data["undo_strategy"]["kind"], "restore_branch_tip_snapshot");

    assert_eq!(
        for_each_ref(&repo),
        before_refs,
        "preview must not change any ref"
    );
}

#[test]
fn executable_plan_resolves_instructions_and_summary() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"fix(login): typo"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );

    let json = json_output(preview_with_instructions(
        &repo,
        "main",
        &instructions_doc(&items),
    ));

    let data = &json["data"];
    assert_eq!(data["execution"]["status"], "executable");
    assert_eq!(data["execution"]["execute_supported"], true);
    assert_eq!(data["execution"]["requires_confirmation_artifact"], false);
    assert_eq!(data["instructions"]["items"][1]["op"], "reword");
    assert_eq!(
        data["instructions"]["items"][1]["message"],
        "fix(login): typo\n"
    );
    // Frozen full object ids, not the abbreviations the agent may have sent.
    assert_eq!(data["instructions"]["items"][0]["commit"], oids[0]);
    assert_eq!(data["result_summary"]["commits_before"], 3);
    assert_eq!(data["result_summary"]["commits_after"], 2);
    assert_eq!(data["result_summary"]["final_tree_unchanged"], true);
    assert_eq!(data["risk"]["severity"], "medium");
    assert_eq!(data["undo_preview"]["available_after_execute"], true);
}

#[test]
fn reorder_preview_reports_order_prediction_and_execute_support() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[1], oids[0], oids[2]
    );

    let json = json_output(preview_with_instructions(
        &repo,
        "main",
        &instructions_doc(&items),
    ));

    let data = &json["data"];
    assert_eq!(data["execution"]["status"], "executable");
    assert_eq!(data["execution"]["execute_supported"], true);
    assert_eq!(data["execution"]["requires_confirmation_artifact"], false);
    assert!(data.get("confirmation").is_none());
    assert!(blocked_codes(data).is_empty());
    assert_eq!(data["reorder"]["commits_reordered"], 2);
    assert_eq!(data["reorder"]["old_order"][0], oids[0]);
    assert_eq!(data["reorder"]["new_order"][0], oids[1]);
    assert_eq!(data["prediction"]["kind"], "reordered_commit_replay");
    assert_eq!(data["prediction"]["status"], "clean");
    assert_eq!(
        data["prediction"]["steps"].as_array().expect("steps").len(),
        3
    );
    assert_eq!(data["prediction"]["dropped_commits"], serde_json::json!([]));
    assert!(data["prediction"]["final_tree"].is_string());
    assert_eq!(data["result_summary"]["final_tree_unchanged"], true);
    assert_eq!(data["undo_strategy"]["kind"], "restore_branch_tip_snapshot");
    assert_eq!(data["undo_preview"]["available_after_execute"], true);
}

#[test]
fn published_range_marks_preview_only_and_confirmation() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    // A remote-tracking ref is enough for the published scan; no network.
    git(
        &repo,
        &["update-ref", "refs/remotes/origin/feature/login", &oids[2]],
    );
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"fix: typo"}},{{"commit":"{}","op":"pick"}}]"#,
        oids[0], oids[1], oids[2]
    );

    let json = json_output(preview_with_instructions(
        &repo,
        "main",
        &instructions_doc(&items),
    ));

    let data = &json["data"];
    assert_eq!(data["execution"]["status"], "preview_only");
    assert_eq!(data["execution"]["requires_confirmation_artifact"], true);
    assert_eq!(data["risk"]["severity"], "high");
    assert_eq!(data["risk"]["requires_human_confirmation"], true);
    assert_eq!(data["confirmation"]["required_before_execute"], true);
    assert!(data["confirmation"]["reason_codes"]
        .as_array()
        .expect("reason codes")
        .iter()
        .any(|code| code == "rewrites_published_commits"));
    assert_eq!(
        data["published_scan"]["published_commits"]
            .as_array()
            .expect("published")
            .len(),
        3
    );
}

#[test]
fn incomplete_instructions_return_blocked_plan_with_repairable_codes() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    // Drops the last commit, so the list does not cover the range.
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}}]"#,
        oids[0], oids[1]
    );

    let json = json_output(preview_with_instructions(
        &repo,
        "main",
        &instructions_doc(&items),
    ));

    let data = &json["data"];
    assert_eq!(data["execution"]["status"], "blocked");
    assert_eq!(data["instructions"], serde_json::Value::Null);
    assert!(blocked_codes(data).contains(&"instructions_incomplete"));
    let missing = data["execution"]["blocked_reasons"]
        .as_array()
        .expect("reasons")
        .iter()
        .find(|reason| reason["code"] == "instructions_incomplete")
        .expect("incomplete reason");
    assert_eq!(missing["details"]["missing_commits"][0], oids[2]);
}

#[test]
fn detached_head_returns_blocked_plan_with_null_branch() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, _) = feature_repo(tmp.path());
    git(&repo, &["checkout", "-q", "--detach"]);

    let json = json_output(preview_survey(&repo, "main"));

    let data = &json["data"];
    assert_eq!(data["execution"]["status"], "blocked");
    assert_eq!(data["branch"], serde_json::Value::Null);
    assert!(blocked_codes(data).contains(&"head_detached"));
}

#[test]
fn malformed_instructions_schema_fails_with_structured_error() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, _) = feature_repo(tmp.path());
    let bad =
        r#"{"schema_version":"super-git.instructions.v9.9","action":"history_edit","items":[]}"#;

    let json = error_json(preview_with_instructions(&repo, "main", bad));

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("instructions_schema_unsupported")));
}

#[test]
fn dirty_tree_warns_but_stays_surveyable() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, _) = feature_repo(tmp.path());
    std::fs::write(repo.join("a.txt"), "changed\n").expect("dirty file");

    let json = json_output(preview_survey(&repo, "main"));

    let data = &json["data"];
    assert_eq!(data["execution"]["status"], "survey");
    assert!(data["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .any(|warning| warning["code"] == "working_tree_dirty"));
}

#[test]
fn human_output_names_status_and_undo_strategy() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, _) = feature_repo(tmp.path());

    let output = super_git(&repo)
        .args(["--human", "preview", "history-edit", "--base", "main"])
        .output()
        .expect("run human survey");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("Execution: survey"));
    assert!(stdout.contains("Undo strategy: restore_branch_tip_snapshot"));
    assert!(stdout.contains("Writes now: no"));
}

#[test]
fn plan_id_is_stable_across_identical_previews() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());
    let items = format!(
        r#"[{{"commit":"{}","op":"pick"}},{{"commit":"{}","op":"reword","message":"m"}},{{"commit":"{}","op":"fixup"}}]"#,
        oids[0], oids[1], oids[2]
    );
    let doc = instructions_doc(&items);

    let first = json_output(preview_with_instructions(&repo, "main", &doc));
    let second = json_output(preview_with_instructions(&repo, "main", &doc));

    assert_eq!(first["data"]["plan_id"], second["data"]["plan_id"]);
}

#[test]
fn survey_instructions_template_round_trips_to_executable_plan() {
    let tmp = tempfile::tempdir().expect("temp");
    let (repo, oids) = feature_repo(tmp.path());

    let survey = json_output(preview_survey(&repo, "main"));
    let template = &survey["data"]["instructions_template"];

    // The template is a complete, valid instructions document: every range
    // commit prefilled as pick, oldest first.
    assert_eq!(template["schema_version"], "super-git.instructions.v0.1");
    assert_eq!(template["action"], "history_edit");
    assert_eq!(template["base"], "main");
    let items = template["items"].as_array().expect("template items");
    assert_eq!(items.len(), 3);
    for (item, oid) in items.iter().zip(&oids) {
        assert_eq!(item["commit"], oid.as_str());
        assert_eq!(item["op"], "pick");
    }

    // The intended agent flow: copy the template, change one op, resubmit --
    // zero schema reconstruction. (Unmodified all-pick input is a no-op and
    // correctly blocks with instructions_no_effective_change.)
    let mut document = template.clone();
    document["items"][1]["op"] = serde_json::json!("reword");
    document["items"][1]["message"] = serde_json::json!("fix(login): correct typo");
    let body = serde_json::to_string(&document).expect("serialize template");
    let json = json_output(preview_with_instructions(&repo, "main", &body));
    assert_eq!(json["data"]["execution"]["status"], "executable");
    assert!(
        json["data"]["instructions_template"].is_null(),
        "non-survey plans carry no template"
    );
}
