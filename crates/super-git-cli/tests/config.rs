//! `super-git config` global app-home contract integration tests.
//! Tests use SUPER_GIT_HOME so they never read or write the real user config.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

fn super_git(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_super-git"));
    cmd.current_dir(dir);
    cmd
}

fn json_output(output: std::process::Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse json")
}

fn error_json(output: std::process::Output) -> serde_json::Value {
    assert!(!output.status.success(), "command should fail");
    serde_json::from_slice(&output.stdout).expect("parse error json")
}

fn path_string(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

fn git(dir: &Path, args: &[&str]) {
    let global_config = dir.join("empty-global-gitconfig");
    let system_config = dir.join("empty-system-gitconfig");
    std::fs::write(&global_config, "").expect("write empty global gitconfig");
    std::fs::write(&system_config, "").expect("write empty system gitconfig");

    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", &global_config)
        .env("GIT_CONFIG_SYSTEM", &system_config)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo_with_commit(repo: &Path) {
    git(repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hello").expect("write file");
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-q", "-m", "initial"]);
}

fn canonical_string(path: &Path) -> String {
    path_string(path.canonicalize().expect("canonical path"))
}

fn read_config_file(app_home: &Path) -> serde_json::Value {
    serde_json::from_str(
        &std::fs::read_to_string(app_home.join("config.json")).expect("read config file"),
    )
    .expect("parse config file")
}

#[test]
fn config_path_uses_super_git_home_without_creating_config_file() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    let json = json_output(
        super_git(tmp.path())
            .args(["config", "path"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config path"),
    );

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["home"], path_string(app_home.clone()));
    assert_eq!(json["data"]["source"], "env:SUPER_GIT_HOME");
    assert_eq!(
        json["data"]["config_file"],
        path_string(app_home.join("config.json"))
    );
    assert!(
        !app_home.join("config.json").exists(),
        "config path must not create config.json"
    );
}

#[test]
fn config_path_uses_project_dirs_without_super_git_home() {
    let tmp = tempfile::tempdir().expect("temp dir");

    let json = json_output(
        super_git(tmp.path())
            .args(["config", "path"])
            .env_remove("SUPER_GIT_HOME")
            .output()
            .expect("run config path"),
    );

    let home = PathBuf::from(json["data"]["home"].as_str().expect("home string"));
    let config_file = PathBuf::from(
        json["data"]["config_file"]
            .as_str()
            .expect("config file string"),
    );

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["source"], "project_dirs");
    assert!(home.is_absolute(), "project_dirs home should be absolute");
    assert_eq!(config_file, home.join("config.json"));
}

#[test]
fn config_path_rejects_empty_super_git_home() {
    let tmp = tempfile::tempdir().expect("temp dir");

    let json = error_json(
        super_git(tmp.path())
            .args(["config", "path"])
            .env("SUPER_GIT_HOME", "")
            .output()
            .expect("run config path"),
    );

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("SUPER_GIT_HOME is set but empty")));
}

#[test]
fn config_path_rejects_relative_super_git_home() {
    let tmp = tempfile::tempdir().expect("temp dir");

    let json = error_json(
        super_git(tmp.path())
            .args(["config", "path"])
            .env("SUPER_GIT_HOME", "relative-home")
            .output()
            .expect("run config path"),
    );

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("SUPER_GIT_HOME must be an absolute path")));
}

#[test]
fn config_show_uses_super_git_home_and_returns_current_config() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    let json = json_output(
        super_git(tmp.path())
            .args(["config", "show"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config show"),
    );

    assert_eq!(json["ok"], true);
    assert_eq!(
        json["data"]["location"]["home"],
        path_string(app_home.clone())
    );
    assert_eq!(json["data"]["location"]["source"], "env:SUPER_GIT_HOME");
    assert_eq!(
        json["data"]["location"]["config_file"],
        path_string(app_home.join("config.json"))
    );
    assert_eq!(
        json["data"]["config"]["repositories"],
        serde_json::json!([])
    );
    assert_eq!(json["data"]["config"]["schema_version"], 1);
    assert_eq!(
        json["data"]["config"]["settings"]["worktree"]["parent_template"],
        "{main_path}.worktrees"
    );
    assert_eq!(
        json["data"]["config"]["settings"]["worktree"]["name_template"],
        "{repo_name}__{ref_slug}"
    );
    assert_eq!(
        json["data"]["config"]["settings"]["worktree"]["ref_slug_algorithm"],
        "path_safe_v1"
    );
    assert!(
        !app_home.join("config.json").exists(),
        "config show must not create config.json when it is missing"
    );
}

#[test]
fn config_validate_reports_default_config_valid() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    let json = json_output(
        super_git(tmp.path())
            .args(["config", "validate"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config validate"),
    );

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["valid"], true);
    assert_eq!(json["data"]["issues"], serde_json::json!([]));
    assert!(
        !app_home.join("config.json").exists(),
        "config validate must not create config.json"
    );
}

#[test]
fn config_set_worktree_template_updates_v1_config() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    let set = json_output(
        super_git(tmp.path())
            .args([
                "config",
                "set-worktree-template",
                "--parent-template",
                "{main_path}.trees",
                "--name-template",
                "{repo_name}--{ref_slug}",
            ])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config set-worktree-template"),
    );

    assert_eq!(set["ok"], true);
    assert_eq!(set["data"]["changed"], true);
    assert_eq!(
        set["data"]["config"]["settings"]["worktree"]["parent_template"],
        "{main_path}.trees"
    );
    assert_eq!(
        set["data"]["config"]["settings"]["worktree"]["name_template"],
        "{repo_name}--{ref_slug}"
    );
    assert_eq!(
        set["data"]["config"]["settings"]["worktree"]["ref_slug_algorithm"],
        "path_safe_v1"
    );

    let raw_config = read_config_file(&app_home);
    assert_eq!(raw_config["schema_version"], 1);
    assert_eq!(
        raw_config["settings"]["worktree"]["parent_template"],
        "{main_path}.trees"
    );
}

#[test]
fn config_set_worktree_template_partial_update_preserves_omitted_fields() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    let set = json_output(
        super_git(tmp.path())
            .args([
                "config",
                "set-worktree-template",
                "--parent-template",
                "{main_path}.trees",
            ])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config set-worktree-template"),
    );

    assert_eq!(set["ok"], true);
    assert_eq!(set["data"]["changed"], true);
    assert_eq!(
        set["data"]["config"]["settings"]["worktree"]["parent_template"],
        "{main_path}.trees"
    );
    assert_eq!(
        set["data"]["config"]["settings"]["worktree"]["name_template"],
        "{repo_name}__{ref_slug}"
    );
    assert_eq!(
        set["data"]["config"]["settings"]["worktree"]["ref_slug_algorithm"],
        "path_safe_v1"
    );
}

#[test]
fn config_set_worktree_template_default_value_creates_missing_config_file() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    let set = json_output(
        super_git(tmp.path())
            .args([
                "config",
                "set-worktree-template",
                "--parent-template",
                "{main_path}.worktrees",
            ])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config set-worktree-template"),
    );

    assert_eq!(set["ok"], true);
    assert_eq!(set["data"]["changed"], true);
    assert!(
        app_home.join("config.json").exists(),
        "set-worktree-template should persist explicit default settings"
    );

    let raw_config = read_config_file(&app_home);
    assert_eq!(raw_config["schema_version"], 1);
    assert_eq!(
        raw_config["settings"]["worktree"]["parent_template"],
        "{main_path}.worktrees"
    );
}

#[test]
fn config_set_worktree_template_idempotent_update_reports_unchanged() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    let first = json_output(
        super_git(tmp.path())
            .args([
                "config",
                "set-worktree-template",
                "--parent-template",
                "{main_path}.trees",
            ])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run first config set-worktree-template"),
    );
    let second = json_output(
        super_git(tmp.path())
            .args([
                "config",
                "set-worktree-template",
                "--parent-template",
                "{main_path}.trees",
            ])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run second config set-worktree-template"),
    );

    assert_eq!(first["data"]["changed"], true);
    assert_eq!(second["ok"], true);
    assert_eq!(second["data"]["changed"], false);
}

#[test]
fn config_set_worktree_template_requires_at_least_one_option() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    let json = error_json(
        super_git(tmp.path())
            .args(["config", "set-worktree-template"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config set-worktree-template"),
    );

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("required arguments were not provided")));
    assert!(
        !app_home.join("config.json").exists(),
        "parse failures must not write config.json"
    );
}

#[test]
fn config_set_worktree_template_rejects_unknown_variable() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    let json = error_json(
        super_git(tmp.path())
            .args([
                "config",
                "set-worktree-template",
                "--name-template",
                "{repo_name}__{branch}",
            ])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config set-worktree-template"),
    );

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("unknown_template_variable")));
    assert!(
        !app_home.join("config.json").exists(),
        "invalid template updates must not write config.json"
    );
}

#[test]
fn config_set_worktree_template_rejects_invalid_update_without_rewriting_existing_config() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    json_output(
        super_git(tmp.path())
            .args([
                "config",
                "set-worktree-template",
                "--parent-template",
                "{main_path}.trees",
            ])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run initial config set-worktree-template"),
    );
    let before = std::fs::read_to_string(app_home.join("config.json")).expect("read config file");

    let json = error_json(
        super_git(tmp.path())
            .args([
                "config",
                "set-worktree-template",
                "--name-template",
                "{repo_name}/{ref_slug}",
            ])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run invalid config set-worktree-template"),
    );
    let after = std::fs::read_to_string(app_home.join("config.json")).expect("read config file");

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("path_separator_in_name_template")));
    assert_eq!(
        after, before,
        "invalid template updates must leave existing config untouched"
    );
}

#[test]
fn config_validate_reports_invalid_manual_template() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    std::fs::write(
        app_home.join("config.json"),
        r#"{
  "schema_version": 1,
  "settings": {
    "worktree": {
      "parent_template": "{main_path}.worktrees",
      "name_template": "{repo_name}__{branch}",
      "ref_slug_algorithm": "path_safe_v1"
    }
  },
  "repositories": []
}"#,
    )
    .expect("write config");

    let json = json_output(
        super_git(tmp.path())
            .args(["config", "validate"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config validate"),
    );

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["valid"], false);
    assert_eq!(
        json["data"]["issues"][0]["field"],
        "settings.worktree.name_template"
    );
    assert_eq!(
        json["data"]["issues"][0]["code"],
        "unknown_template_variable"
    );
}

#[test]
fn doctor_reports_config_path_from_super_git_home() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");

    let json = json_output(
        super_git(tmp.path())
            .arg("doctor")
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run doctor"),
    );

    assert_eq!(json["ok"], true);
    assert_eq!(
        json["data"]["config_path"],
        path_string(app_home.join("config.json"))
    );
}

#[test]
fn repo_save_and_list_use_super_git_home() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    let normalized_repo = canonical_string(&repo);
    let git_common_dir = canonical_string(&repo.join(".git"));

    let save = json_output(
        super_git(tmp.path())
            .args(["repo", "save"])
            .arg(&repo)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo save"),
    );

    assert_eq!(save["ok"], true);
    assert_eq!(save["data"]["added"], true);
    let saved = &save["data"]["repository"];
    assert!(saved["id"].as_str().expect("id").starts_with("sha256:"));
    assert_eq!(saved["name"], "repo");
    assert_eq!(saved["kind"], "worktree_family");
    assert_eq!(saved["main_worktree"], normalized_repo);
    assert_eq!(saved["git_common_dir"], git_common_dir);
    assert_eq!(saved["saved_from"], normalized_repo);
    assert!(
        app_home.join("config.json").exists(),
        "repo save should write inside SUPER_GIT_HOME"
    );

    let list = json_output(
        super_git(tmp.path())
            .args(["repo", "list"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo list"),
    );

    assert_eq!(list["ok"], true);
    assert_eq!(list["data"]["repositories"][0], *saved);

    let show = json_output(
        super_git(tmp.path())
            .args(["config", "show"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config show"),
    );

    assert_eq!(show["ok"], true);
    assert_eq!(show["data"]["config"]["schema_version"], 1);
    assert_eq!(show["data"]["config"]["repositories"][0], *saved);

    let raw_config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(app_home.join("config.json")).expect("read config file"),
    )
    .expect("parse config file");
    assert_eq!(raw_config["schema_version"], 1);
    assert_eq!(
        raw_config["settings"]["worktree"]["parent_template"],
        "{main_path}.worktrees"
    );
    assert_eq!(raw_config["repositories"][0], *saved);
}

#[test]
fn repo_add_is_compatibility_alias_for_save() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);

    let add = json_output(
        super_git(tmp.path())
            .args(["repo", "add"])
            .arg(&repo)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo add"),
    );

    assert_eq!(add["ok"], true);
    assert_eq!(add["data"]["added"], true);
    assert_eq!(add["data"]["path"], canonical_string(&repo));
    assert_eq!(add["data"]["repository"]["kind"], "worktree_family");
    assert_eq!(
        add["data"]["repository"]["main_worktree"],
        canonical_string(&repo)
    );
}

#[test]
fn repo_save_rewrites_duplicate_legacy_v0_config_to_v1() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    let legacy = serde_json::to_string_pretty(&serde_json::json!({
        "repositories": [
            { "path": repo }
        ]
    }))
    .expect("serialize legacy config");
    std::fs::write(app_home.join("config.json"), legacy).expect("write legacy config");

    let save = json_output(
        super_git(tmp.path())
            .args(["repo", "save"])
            .arg(&repo)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo save"),
    );

    assert_eq!(save["ok"], true);
    assert_eq!(save["data"]["added"], false);

    let raw_config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(app_home.join("config.json")).expect("read config file"),
    )
    .expect("parse config file");

    assert_eq!(raw_config["schema_version"], 1);
    assert_eq!(raw_config["repositories"][0]["kind"], "worktree_family");
    assert!(
        raw_config["repositories"][0].get("path").is_none(),
        "repo save should rewrite legacy path entries to the v1 registry shape"
    );
}

#[test]
fn repo_save_dedupes_legacy_v0_main_and_linked_worktree_paths() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    let main = tmp.path().join("repo");
    std::fs::create_dir(&main).expect("create repo dir");
    init_repo_with_commit(&main);
    let linked = tmp.path().join("repo-feature");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            linked.to_str().unwrap(),
        ],
    );
    let legacy = serde_json::to_string_pretty(&serde_json::json!({
        "repositories": [
            { "path": main },
            { "path": linked }
        ]
    }))
    .expect("serialize legacy config");
    std::fs::write(app_home.join("config.json"), legacy).expect("write legacy config");

    let save = json_output(
        super_git(tmp.path())
            .args(["repo", "save"])
            .arg(&main)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo save"),
    );

    assert_eq!(save["ok"], true);

    let raw_config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(app_home.join("config.json")).expect("read config file"),
    )
    .expect("parse config file");

    let repositories = raw_config["repositories"].as_array().expect("repositories");
    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories[0]["kind"], "worktree_family");
    assert_eq!(repositories[0]["main_worktree"], canonical_string(&main));
}

#[test]
fn repo_save_skips_stale_legacy_v0_paths_when_saving_valid_repo() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    let stale = tmp.path().join("missing-repo");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    let legacy = serde_json::to_string_pretty(&serde_json::json!({
        "repositories": [
            { "path": stale }
        ]
    }))
    .expect("serialize legacy config");
    std::fs::write(app_home.join("config.json"), legacy).expect("write legacy config");

    let save = json_output(
        super_git(tmp.path())
            .args(["repo", "save"])
            .arg(&repo)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo save"),
    );

    assert_eq!(save["ok"], true);
    assert_eq!(save["data"]["added"], true);

    let raw_config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(app_home.join("config.json")).expect("read config file"),
    )
    .expect("parse config file");

    assert_eq!(raw_config["schema_version"], 1);
    assert_eq!(
        raw_config["repositories"]
            .as_array()
            .expect("repositories")
            .len(),
        1
    );
    assert_eq!(
        raw_config["repositories"][0]["main_worktree"],
        canonical_string(&repo)
    );
}

#[test]
fn config_show_skips_stale_legacy_v0_paths_in_memory() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    let config_file = app_home.join("config.json");
    let stale = tmp.path().join("missing-repo");
    let legacy = serde_json::to_string_pretty(&serde_json::json!({
        "repositories": [
            { "path": stale }
        ]
    }))
    .expect("serialize legacy config");
    std::fs::write(&config_file, &legacy).expect("write legacy config");

    let json = json_output(
        super_git(tmp.path())
            .args(["config", "show"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config show"),
    );

    assert_eq!(json["ok"], true);
    assert_eq!(
        json["data"]["config"]["repositories"],
        serde_json::json!([])
    );
    assert_eq!(
        std::fs::read_to_string(config_file).expect("read legacy config"),
        legacy,
        "config show must not rewrite legacy files"
    );
}

#[test]
fn repo_add_migrates_legacy_v0_config_to_v1_on_save() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    std::fs::write(
        app_home.join("config.json"),
        r#"{
  "repositories": []
}"#,
    )
    .expect("write legacy config");

    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    let normalized_repo = canonical_string(&repo);

    let add = json_output(
        super_git(tmp.path())
            .args(["repo", "add"])
            .arg(&repo)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo add"),
    );

    assert_eq!(add["ok"], true);

    let raw_config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(app_home.join("config.json")).expect("read config file"),
    )
    .expect("parse config file");

    assert_eq!(raw_config["schema_version"], 1);
    assert_eq!(raw_config["repositories"][0]["kind"], "worktree_family");
    assert_eq!(
        raw_config["settings"]["worktree"]["name_template"],
        "{repo_name}__{ref_slug}"
    );
    assert_eq!(
        raw_config["repositories"][0]["main_worktree"],
        normalized_repo
    );
}

#[test]
fn config_set_worktree_template_migrates_legacy_v0_config_to_v1_on_save() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    let legacy = serde_json::to_string_pretty(&serde_json::json!({
        "repositories": [
            { "path": repo }
        ]
    }))
    .expect("serialize legacy config");
    std::fs::write(app_home.join("config.json"), legacy).expect("write legacy config");

    let set = json_output(
        super_git(tmp.path())
            .args([
                "config",
                "set-worktree-template",
                "--name-template",
                "{repo_name}--{ref_slug}",
            ])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config set-worktree-template"),
    );

    assert_eq!(set["ok"], true);
    assert_eq!(set["data"]["changed"], true);
    assert_eq!(
        set["data"]["config"]["repositories"][0]["kind"],
        "worktree_family"
    );

    let raw_config = read_config_file(&app_home);
    assert_eq!(raw_config["schema_version"], 1);
    assert_eq!(
        raw_config["settings"]["worktree"]["name_template"],
        "{repo_name}--{ref_slug}"
    );
    assert_eq!(raw_config["repositories"][0]["kind"], "worktree_family");
    assert!(
        raw_config["repositories"][0].get("path").is_none(),
        "template updates should rewrite migrated legacy entries to v1 shape"
    );
}

#[test]
fn config_show_migrates_legacy_v0_config_in_memory() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    let config_file = app_home.join("config.json");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    let legacy = serde_json::to_string_pretty(&serde_json::json!({
        "repositories": [
            { "path": repo }
        ]
    }))
    .expect("serialize legacy config");
    std::fs::write(&config_file, &legacy).expect("write legacy config");

    let json = json_output(
        super_git(tmp.path())
            .args(["config", "show"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config show"),
    );

    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["config"]["schema_version"], 1);
    assert_eq!(
        json["data"]["config"]["repositories"][0]["kind"],
        "worktree_family"
    );
    assert_eq!(
        json["data"]["config"]["repositories"][0]["main_worktree"],
        canonical_string(&repo)
    );
    assert_eq!(
        std::fs::read_to_string(config_file).expect("read legacy config"),
        legacy,
        "config show must not rewrite legacy files"
    );
}

#[test]
fn repo_save_from_linked_worktree_uses_family_identity() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let main = tmp.path().join("repo");
    std::fs::create_dir(&main).expect("create repo dir");
    init_repo_with_commit(&main);
    let linked = tmp.path().join("repo-feature");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            linked.to_str().unwrap(),
        ],
    );

    let save_main = json_output(
        super_git(tmp.path())
            .args(["repo", "save"])
            .arg(&main)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo save main"),
    );
    let save_linked = json_output(
        super_git(tmp.path())
            .args(["repo", "save"])
            .arg(&linked)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo save linked"),
    );

    assert_eq!(save_main["data"]["added"], true);
    assert_eq!(save_linked["data"]["added"], false);
    assert_eq!(
        save_main["data"]["repository"]["id"],
        save_linked["data"]["repository"]["id"]
    );
    assert_eq!(
        save_linked["data"]["repository"]["main_worktree"],
        canonical_string(&main)
    );
    assert_eq!(
        save_linked["data"]["repository"],
        save_main["data"]["repository"]
    );

    let list = json_output(
        super_git(tmp.path())
            .args(["repo", "list"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo list"),
    );
    assert_eq!(
        list["data"]["repositories"]
            .as_array()
            .expect("repositories")
            .len(),
        1
    );
    assert_eq!(
        list["data"]["repositories"][0],
        save_main["data"]["repository"]
    );
}

#[test]
fn repo_add_linked_duplicate_keeps_requested_path_in_legacy_field() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let main = tmp.path().join("repo");
    std::fs::create_dir(&main).expect("create repo dir");
    init_repo_with_commit(&main);
    let linked = tmp.path().join("repo-feature");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            linked.to_str().unwrap(),
        ],
    );

    let add_main = json_output(
        super_git(tmp.path())
            .args(["repo", "add"])
            .arg(&main)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo add main"),
    );
    let add_linked = json_output(
        super_git(tmp.path())
            .args(["repo", "add"])
            .arg(&linked)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo add linked"),
    );

    assert_eq!(add_main["data"]["added"], true);
    assert_eq!(add_linked["data"]["added"], false);
    assert_eq!(add_linked["data"]["path"], canonical_string(&linked));
    assert_eq!(
        add_linked["data"]["repository"],
        add_main["data"]["repository"]
    );
}

#[test]
fn repo_add_subdir_keeps_requested_path_in_legacy_field() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    init_repo_with_commit(&repo);
    let subdir = repo.join("nested");
    std::fs::create_dir(&subdir).expect("create nested dir");

    let add = json_output(
        super_git(tmp.path())
            .args(["repo", "add"])
            .arg(&subdir)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo add subdir"),
    );

    assert_eq!(add["ok"], true);
    assert_eq!(add["data"]["path"], canonical_string(&subdir));
    assert_eq!(
        add["data"]["repository"]["main_worktree"],
        canonical_string(&repo)
    );
}

#[test]
fn repo_save_from_bare_primary_worktree_has_no_main_worktree() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let src = tmp.path().join("src");
    std::fs::create_dir(&src).expect("create src dir");
    init_repo_with_commit(&src);

    let bare = tmp.path().join("bare.git");
    git(
        tmp.path(),
        &[
            "clone",
            "--bare",
            "-q",
            src.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let linked = tmp.path().join("linked");
    git(
        &bare,
        &["worktree", "add", "-q", linked.to_str().unwrap(), "main"],
    );

    let save = json_output(
        super_git(tmp.path())
            .args(["repo", "save"])
            .arg(&linked)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo save linked"),
    );

    assert_eq!(save["ok"], true);
    assert_eq!(save["data"]["added"], true);
    let repository = &save["data"]["repository"];
    assert_eq!(repository["kind"], "bare_worktree_family");
    assert_eq!(repository["main_worktree"], serde_json::Value::Null);
    assert_eq!(repository["git_common_dir"], canonical_string(&bare));
    assert_eq!(repository["saved_from"], canonical_string(&linked));
}

#[test]
fn config_show_rejects_unknown_future_schema_version() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    std::fs::write(
        app_home.join("config.json"),
        r#"{
  "schema_version": 999,
  "settings": {},
  "repositories": []
}"#,
    )
    .expect("write future config");

    let json = error_json(
        super_git(tmp.path())
            .args(["config", "show"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config show"),
    );

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("unsupported config schema version: 999")));
}

#[test]
fn config_set_worktree_template_rejects_unknown_future_schema_without_rewriting() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    let config_file = app_home.join("config.json");
    let future = r#"{
  "schema_version": 999,
  "settings": {},
  "repositories": []
}"#;
    std::fs::write(&config_file, future).expect("write future config");

    let json = error_json(
        super_git(tmp.path())
            .args([
                "config",
                "set-worktree-template",
                "--parent-template",
                "{main_path}.trees",
            ])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config set-worktree-template"),
    );

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("unsupported config schema version: 999")));
    assert_eq!(
        std::fs::read_to_string(config_file).expect("read future config"),
        future,
        "future schema files must not be rewritten"
    );
}

#[test]
fn repo_list_rejects_unknown_future_schema_version() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    std::fs::write(
        app_home.join("config.json"),
        r#"{
  "schema_version": 999,
  "settings": {},
  "repositories": []
}"#,
    )
    .expect("write future config");

    let json = error_json(
        super_git(tmp.path())
            .args(["repo", "list"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo list"),
    );

    assert_eq!(json["ok"], false);
    assert!(json["error"]["causes"]
        .as_array()
        .expect("causes array")
        .iter()
        .any(|cause| cause
            .as_str()
            .unwrap_or_default()
            .contains("unsupported_config_schema")));
}
