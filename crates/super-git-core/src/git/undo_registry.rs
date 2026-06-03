use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::model::{UndoRegistryRecord, UndoToken, UNDO_REGISTRY_SCHEMA_VERSION};
use crate::{Result, SuperGitError};

pub fn write_record(token: &UndoToken, undo_dir: &Path) -> Result<PathBuf> {
    let path = record_path_for_snapshot(&token.index_snapshot_path, undo_dir)?;
    let record = UndoRegistryRecord {
        schema_version: UNDO_REGISTRY_SCHEMA_VERSION.to_string(),
        token_sha256: token_sha256(token)?,
        undo_token: token.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&record)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("json.tmp");
    let write_result = (|| -> Result<()> {
        let mut tmp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        tmp.write_all(&bytes)?;
        tmp.sync_all()?;
        drop(tmp);
        fs::rename(&tmp_path, &path)?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(path)
}

pub fn validate_record(token: &UndoToken, undo_dir: &Path) -> Result<()> {
    let path = record_path_for_snapshot(&token.index_snapshot_path, undo_dir)?;
    let metadata = fs::symlink_metadata(&path).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            SuperGitError::UndoTokenInvalid {
                code: "registry_missing".to_string(),
                message: "undo token has no matching local registry record".to_string(),
            }
        } else {
            err.into()
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return invalid_token(
            "unsafe_registry_file",
            "registry record must be a regular file, not a symlink",
        );
    }

    let bytes = fs::read(&path)?;
    let record: UndoRegistryRecord =
        serde_json::from_slice(&bytes).map_err(|err| SuperGitError::UndoTokenInvalid {
            code: "registry_json_invalid".to_string(),
            message: err.to_string(),
        })?;

    if record.schema_version != UNDO_REGISTRY_SCHEMA_VERSION {
        return invalid_token(
            "unsupported_registry_schema_version",
            "undo registry supports only super-git.undo-registry.v0.1",
        );
    }
    if record.undo_token != *token {
        return invalid_token(
            "registry_token_mismatch",
            "undo token does not match the local registry record",
        );
    }

    let expected_hash = token_sha256(token)?;
    if record.token_sha256 != expected_hash {
        return Err(SuperGitError::UndoPreconditionMismatch {
            field: "registry.token_sha256".to_string(),
            expected: expected_hash,
            actual: record.token_sha256,
        });
    }

    Ok(())
}

fn record_path_for_snapshot(snapshot_path: &Path, undo_dir: &Path) -> Result<PathBuf> {
    if !snapshot_path.is_absolute() || !snapshot_path.starts_with(undo_dir) {
        return invalid_token(
            "unsafe_registry_path",
            "snapshot path must stay inside the repository undo directory",
        );
    }

    let relative =
        snapshot_path
            .strip_prefix(undo_dir)
            .map_err(|_| SuperGitError::UndoTokenInvalid {
                code: "unsafe_registry_path".to_string(),
                message: "snapshot path must be under the undo directory".to_string(),
            })?;

    if relative.components().count() != 1
        || !matches!(relative.components().next(), Some(Component::Normal(_)))
    {
        return invalid_token(
            "unsafe_registry_path",
            "snapshot path must be a direct file inside the undo directory",
        );
    }

    let file_name = snapshot_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| SuperGitError::UndoTokenInvalid {
            code: "unsafe_registry_path".to_string(),
            message: "snapshot path must have a valid file name".to_string(),
        })?;
    if !file_name.ends_with(".index") {
        return invalid_token(
            "unsafe_registry_path",
            "snapshot path must use the .index extension",
        );
    }

    let mut path = snapshot_path.to_path_buf();
    path.set_extension("json");
    Ok(path)
}

fn token_sha256(token: &UndoToken) -> Result<String> {
    Ok(sha256_hex(&serde_json::to_vec(token)?))
}

fn invalid_token<T>(code: &str, message: &str) -> Result<T> {
    Err(SuperGitError::UndoTokenInvalid {
        code: code.to_string(),
        message: message.to_string(),
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
