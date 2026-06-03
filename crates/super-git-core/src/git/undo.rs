use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::git::command::Git;
use crate::git::state;
use crate::model::{
    ExecuteResult, UndoResult, UndoToken, UNDO_RESULT_SCHEMA_VERSION, UNDO_TOKEN_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION_STAGE_CHANGES: &str = "stage_changes";

pub fn undo_token_bytes(current_path: &Path, bytes: &[u8]) -> Result<UndoResult> {
    let token = parse_token(bytes)?;
    undo_token(current_path, token)
}

fn parse_token(bytes: &[u8]) -> Result<UndoToken> {
    let value: Value = serde_json::from_slice(bytes)?;
    if value.get("ok").is_some() {
        if value.get("ok") != Some(&Value::Bool(true)) {
            return invalid_token(
                "token_envelope_not_success",
                "token envelope must have ok=true",
            );
        }
        let data = value
            .get("data")
            .ok_or_else(|| SuperGitError::UndoTokenInvalid {
                code: "missing_data".to_string(),
                message: "token envelope must contain data".to_string(),
            })?;

        return token_from_data(data);
    }

    token_from_data(&value)
}

fn token_from_data(value: &Value) -> Result<UndoToken> {
    if let Some(token) = value.get("undo_token") {
        return Ok(serde_json::from_value(token.clone())?);
    }

    if value.get("schema_version").and_then(Value::as_str) == Some(UNDO_TOKEN_SCHEMA_VERSION) {
        return Ok(serde_json::from_value(value.clone())?);
    }

    let result: ExecuteResult = serde_json::from_value(value.clone())?;
    Ok(result.undo_token)
}

fn undo_token(current_path: &Path, token: UndoToken) -> Result<UndoResult> {
    validate_static_token(&token)?;

    let git = Git::default();
    let state = state::read_state(current_path)?;
    ensure_match(
        "repository",
        &token.repository.display().to_string(),
        &state.root.display().to_string(),
    )?;

    let index_path = git_path(&git, &state.root, "index")?;
    let index_lock_path = git_path(&git, &state.root, "index.lock")?;
    let undo_dir = git_path(&git, &state.root, "super-git/undo")?;
    validate_snapshot_path(&token.index_snapshot_path, &undo_dir)?;
    let snapshot = if token.pre_index_existed {
        Some(read_snapshot(&token)?)
    } else {
        None
    };

    let lock = create_index_lock(&index_lock_path)?;
    let current_index_hash = hash_index(&index_path)?;
    if token.post_index_sha256 != current_index_hash {
        drop(lock);
        let _ = fs::remove_file(&index_lock_path);
        return Err(SuperGitError::UndoPreconditionMismatch {
            field: "post_index_sha256".to_string(),
            expected: token.post_index_sha256,
            actual: current_index_hash,
        });
    }
    if let Some(snapshot) = snapshot {
        restore_index(lock, &index_path, &index_lock_path, &snapshot)?;
    } else if index_path.exists() {
        remove_index(lock, &index_path, &index_lock_path)?;
    } else {
        drop(lock);
        fs::remove_file(&index_lock_path)?;
    }

    Ok(UndoResult {
        schema_version: UNDO_RESULT_SCHEMA_VERSION.to_string(),
        action: ACTION_STAGE_CHANGES.to_string(),
        repository: state.root,
        plan_id: token.plan_id,
        undone: true,
        effects: vec!["Restored the pre-execute Git index snapshot.".to_string()],
    })
}

fn validate_static_token(token: &UndoToken) -> Result<()> {
    if token.schema_version != UNDO_TOKEN_SCHEMA_VERSION {
        return invalid_token(
            "unsupported_schema_version",
            "undo supports only super-git.undo.v0.1",
        );
    }
    if token.kind != "restore_index_snapshot" {
        return invalid_token(
            "unsupported_undo_kind",
            "undo supports only restore_index_snapshot",
        );
    }
    if token.action != ACTION_STAGE_CHANGES {
        return invalid_token("unsupported_action", "undo supports only stage_changes");
    }
    if token.target_paths.is_empty() {
        return invalid_token(
            "empty_target_paths",
            "undo token target_paths must not be empty",
        );
    }
    for path in &token.target_paths {
        validate_relative_path(path)?;
    }
    Ok(())
}

fn validate_snapshot_path(snapshot_path: &Path, undo_dir: &Path) -> Result<()> {
    if !snapshot_path.is_absolute() || !snapshot_path.starts_with(undo_dir) {
        return invalid_token(
            "unsafe_snapshot_path",
            "snapshot path must stay inside the repository undo directory",
        );
    }

    let relative =
        snapshot_path
            .strip_prefix(undo_dir)
            .map_err(|_| SuperGitError::UndoTokenInvalid {
                code: "unsafe_snapshot_path".to_string(),
                message: "snapshot path must be under the undo directory".to_string(),
            })?;

    if relative.components().count() != 1
        || !matches!(relative.components().next(), Some(Component::Normal(_)))
    {
        return invalid_token(
            "unsafe_snapshot_path",
            "snapshot path must be a direct file inside the undo directory",
        );
    }

    Ok(())
}

fn validate_relative_path(path: &str) -> Result<()> {
    if path.is_empty() || path.contains('\0') {
        return invalid_token(
            "unsafe_target_path",
            "target path must be a non-empty repository-relative path",
        );
    }

    let mut components = Path::new(path).components();
    let Some(first) = components.next() else {
        return invalid_token(
            "unsafe_target_path",
            "target path must contain at least one normal component",
        );
    };

    if !matches!(first, Component::Normal(_)) || first.as_os_str() == ".git" {
        return invalid_token(
            "unsafe_target_path",
            "target path must stay inside the worktree and outside .git",
        );
    }
    for component in components {
        if !matches!(component, Component::Normal(_)) {
            return invalid_token(
                "unsafe_target_path",
                "target path must not contain absolute, parent, or current-directory components",
            );
        }
    }

    Ok(())
}

fn read_snapshot(token: &UndoToken) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(&token.index_snapshot_path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return invalid_token(
            "unsafe_snapshot_file",
            "snapshot path must be a regular file, not a symlink",
        );
    }

    let bytes = fs::read(&token.index_snapshot_path)?;
    ensure_match(
        "pre_index_sha256",
        &token.pre_index_sha256,
        &sha256_hex(&bytes),
    )?;
    Ok(bytes)
}

fn restore_index(
    mut lock: fs::File,
    index_path: &Path,
    lock_path: &Path,
    snapshot: &[u8],
) -> Result<()> {
    lock.write_all(snapshot)?;
    lock.sync_all()?;
    drop(lock);
    fs::rename(lock_path, index_path)?;
    Ok(())
}

fn remove_index(lock: fs::File, index_path: &Path, lock_path: &Path) -> Result<()> {
    drop(lock);
    match fs::remove_file(index_path) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => {
            let _ = fs::remove_file(lock_path);
            return Err(err.into());
        }
    }
    fs::remove_file(lock_path)?;
    Ok(())
}

fn create_index_lock(lock_path: &Path) -> Result<fs::File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
        .map_err(|err| {
            if err.kind() == ErrorKind::AlreadyExists {
                SuperGitError::UndoPreconditionMismatch {
                    field: "index.lock".to_string(),
                    expected: "absent".to_string(),
                    actual: "present".to_string(),
                }
            } else {
                err.into()
            }
        })
}

fn git_path(git: &Git, root: &Path, path: &str) -> Result<PathBuf> {
    let output = git.run_in(
        root,
        ["rev-parse", "--path-format=absolute", "--git-path", path],
    )?;
    Ok(PathBuf::from(output.stdout.trim()))
}

fn hash_index(path: &Path) -> Result<String> {
    match fs::read(path) {
        Ok(bytes) => Ok(sha256_hex(&bytes)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(sha256_hex(&[])),
        Err(err) => Err(err.into()),
    }
}

fn invalid_token<T>(code: &str, message: &str) -> Result<T> {
    Err(SuperGitError::UndoTokenInvalid {
        code: code.to_string(),
        message: message.to_string(),
    })
}

fn ensure_match(field: &str, expected: &str, actual: &str) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    Err(SuperGitError::UndoPreconditionMismatch {
        field: field.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut output = String::with_capacity("sha256:".len() + digest.len() * 2);
    output.push_str("sha256:");
    for byte in digest {
        output.push(hex_char(byte >> 4));
        output.push(hex_char(byte & 0x0f));
    }
    output
}

fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + value - 10) as char,
        _ => unreachable!("nibble is always <= 15"),
    }
}
