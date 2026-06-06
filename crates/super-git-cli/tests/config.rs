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
fn repo_add_and_list_use_super_git_home() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    git(&repo, &["init", "-q", "-b", "main"]);
    let normalized_repo = repo.canonicalize().expect("canonical repo path");

    let add = json_output(
        super_git(tmp.path())
            .args(["repo", "add"])
            .arg(&repo)
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo add"),
    );

    assert_eq!(add["ok"], true);
    assert_eq!(add["data"]["path"], path_string(normalized_repo.clone()));
    assert_eq!(add["data"]["added"], true);
    assert!(
        app_home.join("config.json").exists(),
        "repo add should write inside SUPER_GIT_HOME"
    );

    let list = json_output(
        super_git(tmp.path())
            .args(["repo", "list"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run repo list"),
    );

    assert_eq!(list["ok"], true);
    assert_eq!(
        list["data"]["repositories"][0]["path"],
        path_string(normalized_repo)
    );

    let show = json_output(
        super_git(tmp.path())
            .args(["config", "show"])
            .env("SUPER_GIT_HOME", &app_home)
            .output()
            .expect("run config show"),
    );

    assert_eq!(show["ok"], true);
    assert_eq!(show["data"]["config"]["schema_version"], 1);
    assert_eq!(
        show["data"]["config"]["repositories"][0]["path"],
        path_string(repo.canonicalize().expect("canonical repo path"))
    );

    let raw_config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(app_home.join("config.json")).expect("read config file"),
    )
    .expect("parse config file");
    assert_eq!(raw_config["schema_version"], 1);
    assert_eq!(
        raw_config["settings"]["worktree"]["parent_template"],
        "{main_path}.worktrees"
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
    git(&repo, &["init", "-q", "-b", "main"]);
    let normalized_repo = repo.canonicalize().expect("canonical repo path");

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
    assert_eq!(
        raw_config["settings"]["worktree"]["name_template"],
        "{repo_name}__{ref_slug}"
    );
    assert_eq!(
        raw_config["repositories"][0]["path"],
        path_string(normalized_repo)
    );
}

#[test]
fn config_show_migrates_legacy_v0_config_in_memory() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let app_home = tmp.path().join("sg-home");
    std::fs::create_dir_all(&app_home).expect("create app home");
    let config_file = app_home.join("config.json");
    let legacy = r#"{
  "repositories": [
    { "path": "/repo/one" }
  ]
}"#;
    std::fs::write(&config_file, legacy).expect("write legacy config");

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
        json["data"]["config"]["repositories"][0]["path"],
        "/repo/one"
    );
    assert_eq!(
        std::fs::read_to_string(config_file).expect("read legacy config"),
        legacy,
        "config show must not rewrite legacy files"
    );
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
