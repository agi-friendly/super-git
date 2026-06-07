//! `super-git preview worktree-create` integration tests.
//! Preview must resolve a typed v0.2 plan without creating worktrees or folders.

use std::path::Path;
use std::process::{Command, Output};

use super_git_core::git::preview_worktree::compute_worktree_plan_id;
use super_git_core::model::WorktreeCreatePlan;

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

fn init_repo_with_commit(repo: &Path) {
    git(repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hello\n").expect("write file");
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "initial"]);
}

fn path_string(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

fn canonical_string(path: impl AsRef<Path>) -> String {
    path_string(path.as_ref().canonicalize().expect("canonical path"))
}

fn canonical_sibling(path: &Path, sibling_name: &str) -> std::path::PathBuf {
    let mut canonical = path.canonicalize().expect("canonical path");
    canonical.set_file_name(sibling_name);
    canonical
}

fn worktree_list(repo: &Path) -> String {
    let output = run_git(repo, &["worktree", "list", "--porcelain"]);
    assert!(output.status.success(), "worktree list should succeed");
    String::from_utf8(output.stdout).expect("worktree list should be utf8")
}

fn preview_worktree(
    dir: &Path,
    app_home: &Path,
    repo: Option<&str>,
    ref_name: &str,
) -> serde_json::Value {
    let mut command = super_git(dir);
    command
        .args(["preview", "worktree-create"])
        .env("SUPER_GIT_HOME", app_home);
    if let Some(repo) = repo {
        command.args(["--repo", repo]);
    }
    command.args(["--ref", ref_name]);

    let output = command.output().expect("run preview worktree-create");
    assert!(
        output.status.success(),
        "preview should succeed with a typed plan: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    serde_json::from_slice(&output.stdout).expect("parse preview json")
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

#[test]
fn preview_worktree_create_from_current_repo_emits_executable_plan_without_writes() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "works/eml-base"]);
    let target_parent = canonical_sibling(&repo, "repo.worktrees");
    let before_worktrees = worktree_list(&repo);

    let json = preview_worktree(&repo, &app_home, None, "works/eml-base");

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["schema_version"], "super-git.plan.v0.2");
    assert!(data["plan_id"]
        .as_str()
        .expect("plan id")
        .starts_with("sha256:"));
    assert_eq!(data["action"]["kind"], "worktree_create");
    assert_eq!(
        data["action"]["options"]["repo_selector"],
        serde_json::Value::Null
    );
    assert_eq!(data["action"]["options"]["ref"], "works/eml-base");
    assert_eq!(data["repository"]["main_worktree"], canonical_string(&repo));
    assert_eq!(data["repository"]["selected_from"], canonical_string(&repo));
    assert_eq!(data["source_ref"]["kind"], "local_branch");
    assert_eq!(data["source_ref"]["full_ref"], "refs/heads/works/eml-base");
    assert_eq!(data["source_ref"]["supported_for_execute"], true);
    assert_eq!(data["ref_policy"]["mode"], "existing_local_branch");
    assert_eq!(data["target"]["parent"], path_string(&target_parent));
    assert_eq!(data["target"]["name"], "repo__works-eml-base");
    assert_eq!(data["target"]["exists"], false);
    assert_eq!(data["target"]["parent_creation"]["will_create"], true);
    assert_eq!(data["execution"]["status"], "executable");
    assert_eq!(
        data["execution"]["suggested_super_git_command"],
        serde_json::json!(["super-git", "execute", "--plan", "<plan-file>"])
    );
    assert_eq!(data["execution"]["raw_git_allowed"], false);
    assert_eq!(
        data["reference_commands"]["semantics"],
        "documentation_only"
    );
    assert_eq!(data["reference_commands"]["never_execute_directly"], true);
    assert!(data["family_snapshot"]["fingerprint"]
        .as_str()
        .expect("family fingerprint")
        .starts_with("sha256:"));
    assert!(data["family_snapshot"]["worktrees"]
        .as_array()
        .expect("worktrees")
        .iter()
        .any(|entry| entry["kind"] == "main"));

    assert_eq!(
        worktree_list(&repo),
        before_worktrees,
        "preview must not mutate Git worktree metadata"
    );
    assert!(
        !target_parent.exists(),
        "preview must not create the target parent directory"
    );
    assert!(
        !app_home.join("config.json").exists(),
        "preview must not create config.json"
    );
}

#[test]
fn preview_worktree_create_accepts_saved_repository_name_selector() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::create_dir_all(&outside).expect("create outside dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/name-selector"]);

    let save = super_git(&outside)
        .args(["repo", "save"])
        .arg(&repo)
        .env("SUPER_GIT_HOME", &app_home)
        .output()
        .expect("save repo");
    assert!(
        save.status.success(),
        "repo save failed: {}",
        String::from_utf8_lossy(&save.stderr)
    );

    let json = preview_worktree(&outside, &app_home, Some("repo"), "feature/name-selector");

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["execution"]["status"], "executable");
    assert_eq!(data["action"]["options"]["repo_selector"], "repo");
    assert_eq!(data["repository"]["selected_from"], canonical_string(&repo));
    assert_eq!(data["target"]["name"], "repo__feature-name-selector");
}

#[test]
fn preview_worktree_create_accepts_saved_repository_id_and_path_selectors() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::create_dir_all(&outside).expect("create outside dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/selectors"]);

    let save = json_output(
        super_git(&outside)
            .args(["repo", "save"])
            .arg(&repo)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("save repo"),
    );
    let id = save["data"]["repository"]["id"]
        .as_str()
        .expect("repository id");
    let repo_path = canonical_string(&repo);

    let by_id = preview_worktree(&outside, &app_home, Some(id), "feature/selectors");
    let by_path = preview_worktree(&outside, &app_home, Some(&repo_path), "feature/selectors");

    assert_eq!(by_id["data"]["execution"]["status"], "executable");
    assert_eq!(by_id["data"]["action"]["options"]["repo_selector"], id);
    assert_eq!(by_path["data"]["execution"]["status"], "executable");
    assert_eq!(
        by_path["data"]["action"]["options"]["repo_selector"],
        repo_path
    );
}

#[test]
fn preview_worktree_create_accepts_unsaved_path_selector_without_writing_config() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::create_dir_all(&outside).expect("create outside dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/unsaved-path"]);
    let repo_path = canonical_string(&repo);

    let json = preview_worktree(
        &outside,
        &app_home,
        Some(&repo_path),
        "feature/unsaved-path",
    );

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["execution"]["status"], "executable");
    assert_eq!(
        json["data"]["action"]["options"]["repo_selector"],
        repo_path
    );
    assert_eq!(json["data"]["repository"]["selected_from"], repo_path);
    assert!(
        !app_home.join("config.json").exists(),
        "unsaved path selector preview must not create config.json"
    );
}

#[test]
fn preview_worktree_create_supports_tag_and_commit_as_detached_head_plans() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["tag", "v1.0.0"]);
    let head = run_git(&repo, &["rev-parse", "--short=12", "HEAD"]);
    assert!(head.status.success(), "rev-parse HEAD should succeed");
    let short_commit = String::from_utf8(head.stdout)
        .expect("commit utf8")
        .trim()
        .to_string();

    let tag = preview_worktree(&repo, &app_home, None, "v1.0.0");
    let commit = preview_worktree(&repo, &app_home, None, &short_commit);

    assert_eq!(tag["data"]["source_ref"]["kind"], "tag");
    assert_eq!(tag["data"]["source_ref"]["supported_for_execute"], true);
    assert_eq!(tag["data"]["ref_policy"]["mode"], "detached_head");
    assert_eq!(tag["data"]["ref_policy"]["will_detach_head"], true);
    assert_eq!(tag["data"]["execution"]["status"], "executable");
    assert_eq!(commit["data"]["source_ref"]["kind"], "commit");
    assert_eq!(commit["data"]["source_ref"]["supported_for_execute"], true);
    assert_eq!(commit["data"]["ref_policy"]["mode"], "detached_head");
    assert_eq!(commit["data"]["execution"]["status"], "executable");
}

#[test]
fn preview_worktree_create_blocks_ambiguous_branch_and_tag_ref() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "release"]);
    git(&repo, &["tag", "release"]);

    let json = preview_worktree(&repo, &app_home, None, "release");

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["source_ref"]["kind"], "ambiguous");
    assert_eq!(data["source_ref"]["supported_for_execute"], false);
    assert_eq!(data["execution"]["status"], "blocked");
    assert!(blocked_codes(data).contains(&"source_ref_ambiguous"));
}

#[test]
fn preview_worktree_create_blocks_remote_tracking_branch() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let origin = tmp.path().join("origin.git");
    let repo = tmp.path().join("repo");
    git(
        tmp.path(),
        &["init", "-q", "--bare", origin.to_str().unwrap()],
    );
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            repo.to_str().unwrap(),
        ],
    );
    std::fs::write(repo.join("README.md"), "hello\n").expect("write file");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    git(&repo, &["push", "-q", "-u", "origin", "HEAD:main"]);

    let json = preview_worktree(&repo, &app_home, None, "origin/main");

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["source_ref"]["kind"], "remote_tracking_branch");
    assert_eq!(data["source_ref"]["full_ref"], "refs/remotes/origin/main");
    assert_eq!(data["source_ref"]["supported_for_execute"], false);
    assert_eq!(data["execution"]["status"], "blocked");
    assert!(blocked_codes(data).contains(&"remote_tracking_branch_requires_local_branch_policy"));
}

#[test]
fn preview_worktree_create_blocks_branch_already_checked_out_elsewhere() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    let occupied = tmp.path().join("occupied-feature");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/occupied"]);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            occupied.to_str().unwrap(),
            "feature/occupied",
        ],
    );

    let json = preview_worktree(&repo, &app_home, None, "feature/occupied");

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["source_ref"]["kind"], "local_branch");
    assert_eq!(data["execution"]["status"], "blocked");
    assert!(blocked_codes(data).contains(&"branch_already_checked_out"));
    assert!(data["family_snapshot"]["branch_occupancy"]
        .as_array()
        .expect("branch occupancy array")
        .iter()
        .any(|entry| entry["branch"] == "refs/heads/feature/occupied"));
}

#[test]
fn preview_worktree_create_blocks_target_collision_without_deleting_it() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/collision"]);
    let target = canonical_sibling(&repo, "repo.worktrees").join("repo__feature-collision");
    std::fs::create_dir_all(&target).expect("create colliding target");

    let json = preview_worktree(&repo, &app_home, None, "feature/collision");

    assert_eq!(json["ok"], true);
    let data = &json["data"];
    assert_eq!(data["target"]["path"], path_string(&target));
    assert_eq!(data["target"]["exists"], true);
    assert_eq!(data["execution"]["status"], "blocked");
    assert!(blocked_codes(data).contains(&"target_path_exists"));
    assert!(target.exists(), "preview must not remove colliding paths");
}

#[test]
fn preview_worktree_create_rejects_invalid_config_before_plan() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    std::fs::create_dir_all(&app_home).expect("create app home");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/invalid-config"]);
    std::fs::write(
        app_home.join("config.json"),
        r#"{
  "schema_version": 1,
  "settings": {
    "worktree": {
      "parent_template": "outside-worktrees",
      "name_template": "{repo_name}__{ref_slug}",
      "ref_slug_algorithm": "path_safe_v1"
    }
  },
  "repositories": []
}"#,
    )
    .expect("write invalid config");

    let output = super_git(&repo)
        .args([
            "preview",
            "worktree-create",
            "--ref",
            "feature/invalid-config",
        ])
        .env("SUPER_GIT_HOME", &app_home)
        .output()
        .expect("run preview worktree-create");
    let json = error_json(output);

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("missing_required_variable")));
}

fn execute_worktree_plan(dir: &Path, app_home: &Path, plan: &serde_json::Value) -> Output {
    let mut command = super_git(dir);
    command
        .args(["execute", "--plan", "-"])
        .env("SUPER_GIT_HOME", app_home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().expect("spawn execute");
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("stdin");
        stdin
            .write_all(
                serde_json::to_string(plan)
                    .expect("serialize plan")
                    .as_bytes(),
            )
            .expect("write plan to stdin");
    }
    child.wait_with_output().expect("wait execute")
}

fn rehash_worktree_plan(envelope: &mut serde_json::Value) {
    let mut plan: WorktreeCreatePlan =
        serde_json::from_value(envelope["data"].clone()).expect("parse worktree plan");
    plan.plan_id = compute_worktree_plan_id(&plan).expect("worktree plan id");
    envelope["data"] = serde_json::to_value(plan).expect("serialize worktree plan");
}

#[test]
fn execute_worktree_create_creates_linked_worktree_and_returns_undo_token() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/execute-boundary"]);
    let plan = preview_worktree(&repo, &app_home, None, "feature/execute-boundary");
    let target = std::path::PathBuf::from(
        plan["data"]["target"]["path"]
            .as_str()
            .expect("target path"),
    );

    let output = execute_worktree_plan(&repo, &app_home, &plan);
    let json = json_output(output);

    assert_eq!(json["data"]["action"], "worktree_create");
    assert_eq!(
        json["data"]["undo_token"]["kind"],
        "remove_created_worktree_if_clean"
    );
    assert_eq!(
        json["data"]["undo_token"]["target_path"],
        path_string(&target)
    );
    assert!(
        std::path::PathBuf::from(
            json["data"]["undo_token"]["execution_record_path"]
                .as_str()
                .expect("execution record path")
        )
        .exists(),
        "execute must write local provenance before returning a token"
    );
    assert!(target.join(".git").exists(), "target worktree should exist");
    let branch = run_git(&target, &["branch", "--show-current"]);
    assert!(
        branch.status.success(),
        "new worktree branch should be readable"
    );
    assert_eq!(
        String::from_utf8(branch.stdout)
            .expect("branch utf8")
            .trim(),
        "feature/execute-boundary"
    );
    assert!(worktree_list(&repo).contains(&path_string(&target)));
}

#[test]
fn execute_worktree_create_revalidates_branch_occupancy_before_writing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    let occupied = tmp.path().join("occupied-feature");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/stale-occupancy"]);
    let plan = preview_worktree(&repo, &app_home, None, "feature/stale-occupancy");
    let target = std::path::PathBuf::from(
        plan["data"]["target"]["path"]
            .as_str()
            .expect("target path"),
    );
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            occupied.to_str().unwrap(),
            "feature/stale-occupancy",
        ],
    );

    let output = execute_worktree_plan(&repo, &app_home, &plan);
    let json = error_json(output);

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("family_snapshot.fingerprint")));
    assert!(
        !target.exists(),
        "execute must fail before creating a stale target"
    );
}

#[test]
fn execute_worktree_create_revalidates_source_ref_before_writing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/source-moved"]);
    let plan = preview_worktree(&repo, &app_home, None, "feature/source-moved");
    let target = std::path::PathBuf::from(
        plan["data"]["target"]["path"]
            .as_str()
            .expect("target path"),
    );
    let new_commit = run_git(
        &repo,
        &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "move"],
    );
    assert!(new_commit.status.success(), "commit-tree should succeed");
    let new_commit = String::from_utf8(new_commit.stdout)
        .expect("commit-tree utf8")
        .trim()
        .to_string();
    git(
        &repo,
        &["update-ref", "refs/heads/feature/source-moved", &new_commit],
    );

    let output = execute_worktree_plan(&repo, &app_home, &plan);
    let json = error_json(output);

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("source_ref.resolved_commit")));
    assert!(!target.exists(), "execute must fail before creating target");
}

#[test]
fn execute_worktree_create_rechecks_target_collision_without_deleting_it() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/stale-target"]);
    let plan = preview_worktree(&repo, &app_home, None, "feature/stale-target");
    let target = std::path::PathBuf::from(
        plan["data"]["target"]["path"]
            .as_str()
            .expect("target path"),
    );
    std::fs::create_dir_all(&target).expect("create stale target collision");

    let output = execute_worktree_plan(&repo, &app_home, &plan);
    let json = error_json(output);

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("target_path_available")));
    assert!(target.exists(), "execute must not delete colliding paths");
}

#[test]
fn execute_worktree_create_rejects_tampered_plan_id_before_writing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/tampered"]);
    let mut plan = preview_worktree(&repo, &app_home, None, "feature/tampered");
    let target = std::path::PathBuf::from(
        plan["data"]["target"]["path"]
            .as_str()
            .expect("target path"),
    );
    plan["data"]["plan_id"] = serde_json::json!("sha256:tampered");

    let output = execute_worktree_plan(&repo, &app_home, &plan);
    let json = error_json(output);

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause.as_str().unwrap_or_default().contains("plan_id")));
    assert!(!target.exists(), "execute must fail before creating target");
}

#[test]
fn execute_worktree_create_rejects_blocked_plan_before_writing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let origin = tmp.path().join("origin.git");
    let repo = tmp.path().join("repo");
    git(
        tmp.path(),
        &["init", "-q", "--bare", origin.to_str().unwrap()],
    );
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            repo.to_str().unwrap(),
        ],
    );
    std::fs::write(repo.join("README.md"), "hello\n").expect("write file");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    git(&repo, &["push", "-q", "-u", "origin", "HEAD:main"]);
    let plan = preview_worktree(&repo, &app_home, None, "origin/main");

    let output = execute_worktree_plan(&repo, &app_home, &plan);
    let json = error_json(output);

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("not_executable")));
}

#[test]
fn execute_worktree_create_ignores_advisory_reference_commands() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/advisory"]);
    let mut plan = preview_worktree(&repo, &app_home, None, "feature/advisory");
    plan["data"]["reference_commands"] = serde_json::json!({
        "semantics": "documentation_only",
        "never_execute_directly": true,
        "commands": [["sh", "-c", "touch pwned"], ["git", "reset", "--hard"]]
    });
    rehash_worktree_plan(&mut plan);

    let output = execute_worktree_plan(&repo, &app_home, &plan);
    let json = json_output(output);

    assert_eq!(json["data"]["action"], "worktree_create");
    assert!(
        !repo.join("pwned").exists(),
        "execute must rebuild trusted git args, not run reference_commands"
    );
}

#[test]
fn execute_worktree_create_rejects_rehashed_inconsistent_ref_policy_before_writing() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    git(&repo, &["branch", "feature/ref-policy"]);
    let mut plan = preview_worktree(&repo, &app_home, None, "feature/ref-policy");
    let target = std::path::PathBuf::from(
        plan["data"]["target"]["path"]
            .as_str()
            .expect("target path"),
    );
    plan["data"]["ref_policy"]["mode"] = serde_json::json!("detached_head");
    plan["data"]["ref_policy"]["will_detach_head"] = serde_json::json!(true);
    rehash_worktree_plan(&mut plan);

    let output = execute_worktree_plan(&repo, &app_home, &plan);
    let json = error_json(output);

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("ref_policy_consistency")));
    assert!(!target.exists(), "execute must fail before creating target");
}

fn blocked_codes(data: &serde_json::Value) -> Vec<&str> {
    data["execution"]["blocked_reasons"]
        .as_array()
        .expect("blocked reasons")
        .iter()
        .map(|reason| reason["code"].as_str().expect("blocked code"))
        .collect()
}
