use std::fs;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::git::command::Git;
use crate::model::{Operation, StateFingerprint, FINGERPRINT_SCHEMA_VERSION};
use crate::{Result, SuperGitError};

/// Adapter so a file can be streamed into a Sha256 hasher with io::copy instead
/// of buffering the whole file into memory.
struct HashWriter<'a>(&'a mut Sha256);

impl io::Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

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
        // --untracked-files=all pins the mode so status.showUntrackedFiles=no
        // cannot make the fingerprint blind to untracked changes.
        status_porcelain_v1_z_sha256: hash_git_output(
            git,
            path,
            ["status", "--porcelain=v1", "--untracked-files=all", "-z"],
        )?,
        // --no-ext-diff/--no-textconv keep the fingerprint bound to the actual
        // content. Otherwise a repo with `diff.external` (e.g. difftastic) or a
        // textconv driver substitutes the driver's output — often a per-call
        // temp path — so preview and execute hash different bytes and every
        // execute fails with a state_fingerprint mismatch. They also stop the
        // fingerprint from being a side channel that runs an external command.
        staged_diff_sha256: hash_git_output(
            git,
            path,
            [
                "diff",
                "--cached",
                "--no-ext-diff",
                "--no-textconv",
                "--binary",
                "--full-index",
            ],
        )?,
        unstaged_diff_sha256: hash_git_output(
            git,
            path,
            [
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--binary",
                "--full-index",
            ],
        )?,
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
        let entry = read_untracked_entry(&relative_path, &full_path)?;
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

#[derive(Debug)]
struct UntrackedEntry {
    kind: &'static str,
    mode: String,
    len: usize,
    content_hash: String,
}

fn read_untracked_entry(relative_path: &str, path: &Path) -> Result<UntrackedEntry> {
    let metadata = fs::symlink_metadata(path).map_err(|err| unreadable(relative_path, err))?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|err| unreadable(relative_path, err))?;
        let bytes = target.as_os_str().as_encoded_bytes();
        return Ok(UntrackedEntry {
            kind: "symlink",
            mode: mode_string(&metadata),
            len: bytes.len(),
            content_hash: sha256_hex(bytes),
        });
    }

    // Stream the file into the hasher instead of buffering it: a large untracked
    // file (a dataset or video) would otherwise be read fully into memory on
    // every preview and execute revalidation.
    let file = fs::File::open(path).map_err(|err| unreadable(relative_path, err))?;
    let mut hasher = Sha256::new();
    let len = io::copy(&mut io::BufReader::new(file), &mut HashWriter(&mut hasher))
        .map_err(|err| unreadable(relative_path, err))?;
    Ok(UntrackedEntry {
        kind: "file",
        mode: mode_string(&metadata),
        len: len as usize,
        content_hash: format_digest(hasher.finalize().as_slice()),
    })
}

/// Wrap a bare io error (which carries no path) with the relative path and phase,
/// so a file vanishing between `ls-files` and the read is identifiable instead
/// of surfacing as a context-free ENOENT.
fn unreadable(relative_path: &str, err: io::Error) -> SuperGitError {
    SuperGitError::PreviewPreconditionFailed {
        action: "fingerprint".to_string(),
        code: "untracked_entry_unreadable".to_string(),
        message: format!("could not read untracked entry {relative_path}: {err}"),
    }
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
    use super::{parse_z_paths, read_untracked_entry, sha256_hex};

    #[test]
    fn parses_nul_delimited_paths() {
        assert_eq!(
            parse_z_paths(b"file.txt\0dir/new.txt\0"),
            vec!["file.txt", "dir/new.txt"]
        );
    }

    #[test]
    fn read_untracked_entry_streams_matching_content_hash() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let path = tmp.path().join("data.bin");
        std::fs::write(&path, b"hello untracked").expect("write file");

        let entry = read_untracked_entry("data.bin", &path).expect("read entry");

        assert_eq!(entry.kind, "file");
        assert_eq!(entry.len, 15);
        // Streaming the file must produce the same digest as hashing it whole.
        assert_eq!(entry.content_hash, sha256_hex(b"hello untracked"));
    }

    #[test]
    fn read_untracked_entry_missing_file_names_the_path() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let path = tmp.path().join("vanished.txt");

        let err = read_untracked_entry("sub/vanished.txt", &path).expect_err("missing file errors");

        assert!(
            err.to_string().contains("sub/vanished.txt"),
            "error must name the path: {err}"
        );
    }
}
