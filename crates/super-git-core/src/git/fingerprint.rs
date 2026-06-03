use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::git::command::Git;
use crate::model::{Operation, StateFingerprint, FINGERPRINT_SCHEMA_VERSION};
use crate::Result;

pub fn read_state_fingerprint(
    git: &Git,
    path: &Path,
    repository: &Path,
    head_commit: Option<String>,
    operation: Operation,
) -> Result<StateFingerprint> {
    Ok(StateFingerprint {
        schema_version: FINGERPRINT_SCHEMA_VERSION.to_string(),
        repository: repository.to_path_buf(),
        head_commit,
        operation,
        status_porcelain_v1_z_sha256: hash_git_output(
            git,
            path,
            ["status", "--porcelain=v1", "-z"],
        )?,
        staged_diff_sha256: hash_git_output(
            git,
            path,
            ["diff", "--cached", "--binary", "--full-index"],
        )?,
        unstaged_diff_sha256: hash_git_output(git, path, ["diff", "--binary", "--full-index"])?,
        untracked_content_sha256: hash_untracked_content(git, path, repository)?,
    })
}

pub fn resolved_stage_changes_paths(git: &Git, path: &Path) -> Result<Vec<String>> {
    let mut paths = parse_z_paths(
        &git.run_bytes_in(path, ["diff", "--name-only", "-z"])?
            .stdout,
    );
    paths.extend(parse_z_paths(
        &git.run_bytes_in(path, ["ls-files", "--others", "--exclude-standard", "-z"])?
            .stdout,
    ));
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn hash_git_output<I, S>(git: &Git, path: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Ok(sha256_hex(&git.run_bytes_in(path, args)?.stdout))
}

fn hash_untracked_content(git: &Git, path: &Path, repository: &Path) -> Result<String> {
    let mut paths = parse_z_paths(
        &git.run_bytes_in(path, ["ls-files", "--others", "--exclude-standard", "-z"])?
            .stdout,
    );
    paths.sort();

    let mut hasher = Sha256::new();
    for relative_path in paths {
        let full_path = repository.join(&relative_path);
        let entry = read_untracked_entry(&full_path)?;
        hasher.update(relative_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(entry.kind.as_bytes());
        hasher.update(b"\0");
        hasher.update(entry.mode.as_bytes());
        hasher.update(b"\0");
        hasher.update(entry.len.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(entry.content_hash.as_bytes());
        hasher.update(b"\0");
    }

    Ok(format_digest(hasher.finalize().as_slice()))
}

struct UntrackedEntry {
    kind: &'static str,
    mode: String,
    len: usize,
    content_hash: String,
}

fn read_untracked_entry(path: &Path) -> Result<UntrackedEntry> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path)?;
        let bytes = target.as_os_str().as_encoded_bytes();
        return Ok(UntrackedEntry {
            kind: "symlink",
            mode: mode_string(&metadata),
            len: bytes.len(),
            content_hash: sha256_hex(bytes),
        });
    }

    let bytes = fs::read(path)?;
    Ok(UntrackedEntry {
        kind: "file",
        mode: mode_string(&metadata),
        len: bytes.len(),
        content_hash: sha256_hex(&bytes),
    })
}

#[cfg(unix)]
fn mode_string(metadata: &fs::Metadata) -> String {
    format!("{:o}", metadata.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn mode_string(_metadata: &fs::Metadata) -> String {
    "portable".to_string()
}

fn parse_z_paths(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).to_string())
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format_digest(hasher.finalize().as_slice())
}

fn format_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity("sha256:".len() + bytes.len() * 2);
    output.push_str("sha256:");
    for byte in bytes {
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

#[cfg(test)]
mod tests {
    use super::parse_z_paths;

    #[test]
    fn parses_nul_delimited_paths() {
        assert_eq!(
            parse_z_paths(b"file.txt\0dir/new.txt\0"),
            vec!["file.txt", "dir/new.txt"]
        );
    }
}
