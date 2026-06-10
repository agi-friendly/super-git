use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::{Result, SuperGitError};

/// Decode a single path printed by git, trimming one trailing newline. On unix
/// the bytes become an OsString verbatim (non-UTF-8 names survive); elsewhere
/// (git emits UTF-8 on Windows) a lossy string is used.
fn decode_git_path(stdout: &[u8]) -> PathBuf {
    let trimmed = stdout.strip_suffix(b"\n").unwrap_or(stdout);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(OsString::from_vec(trimmed.to_vec()))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(trimmed).into_owned())
    }
}

#[derive(Debug, Clone)]
pub struct Git {
    program: OsString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitBytesOutput {
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Default for Git {
    fn default() -> Self {
        Self {
            program: OsString::from("git"),
        }
    }
}

impl Git {
    pub fn version(&self) -> Result<String> {
        let output = self.run(["--version"])?;
        Ok(output.stdout.trim().to_string())
    }

    pub fn run<I, S>(&self, args: I) -> Result<GitOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = collect_args(args);
        let output = self.read_command().args(&args).output()?;
        self.output_or_error(display_args(&args), output)
    }

    pub fn run_in<I, S>(&self, path: &Path, args: I) -> Result<GitOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = collect_args(args);
        let output = self
            .read_command()
            .arg("-C")
            .arg(path)
            .args(&args)
            .output()?;

        let mut shown_args = vec!["-C".to_string(), path.display().to_string()];
        shown_args.extend(display_args(&args));

        self.output_or_error(shown_args, output)
    }

    pub fn run_bytes_in<I, S>(&self, path: &Path, args: I) -> Result<GitBytesOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = collect_args(args);
        let output = self
            .read_command()
            .arg("-C")
            .arg(path)
            .args(&args)
            .output()?;

        let mut shown_args = vec!["-C".to_string(), path.display().to_string()];
        shown_args.extend(display_args(&args));

        self.bytes_output_or_error(shown_args, output)
    }

    pub fn run_write_in<I, S>(&self, path: &Path, args: I) -> Result<GitOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = collect_args(args);
        let output = self
            .write_command()
            .arg("-C")
            .arg(path)
            .args(&args)
            .output()?;

        let mut shown_args = vec!["-C".to_string(), path.display().to_string()];
        shown_args.extend(display_args(&args));

        self.output_or_error(shown_args, output)
    }

    /// Read a single path printed by git (e.g. `rev-parse --show-toplevel`).
    /// On unix the path is built from raw bytes so a non-UTF-8 filename is
    /// preserved exactly, instead of being mangled into U+FFFD by lossy string
    /// conversion (which would yield a path that does not exist on disk).
    pub fn run_path_in<I, S>(&self, path: &Path, args: I) -> Result<PathBuf>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.run_bytes_in(path, args)?;
        Ok(decode_git_path(&output.stdout))
    }

    /// Write-side run with extra environment variables and a stdin payload.
    /// Used by history edit's `commit-tree`, which needs `GIT_AUTHOR_*` to
    /// preserve each rewritten commit's author and reads the message from stdin.
    pub fn run_write_in_with_env_stdin<I, S>(
        &self,
        path: &Path,
        args: I,
        envs: &[(&str, &str)],
        stdin_bytes: &[u8],
    ) -> Result<GitOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = collect_args(args);
        let mut command = self.write_command();
        command.arg("-C").arg(path).args(&args);
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .expect("stdin was requested as piped")
            .write_all(stdin_bytes)?;
        let output = child.wait_with_output()?;

        let mut shown_args = vec!["-C".to_string(), path.display().to_string()];
        shown_args.extend(display_args(&args));
        self.output_or_error(shown_args, output)
    }

    pub fn try_run_in<I, S>(&self, path: &Path, args: I) -> Result<GitCommandResult>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = collect_args(args);
        let output = self
            .read_command()
            .arg("-C")
            .arg(path)
            .args(&args)
            .output()?;

        Ok(GitCommandResult::from_output(output))
    }

    /// Read a boolean git config value. Returns false when the key is unset,
    /// unreadable, or not literally "true" after `--type=bool` normalization.
    pub fn config_bool_true(&self, path: &Path, key: &str) -> bool {
        self.try_run_in(path, ["config", "--type=bool", "--get", key])
            .ok()
            .filter(|output| output.success)
            .is_some_and(|output| output.stdout.trim() == "true")
    }

    fn output_or_error(&self, args: Vec<String>, output: Output) -> Result<GitOutput> {
        let result = GitCommandResult::from_output(output);

        if result.success {
            Ok(GitOutput {
                stdout: result.stdout,
                stderr: result.stderr,
            })
        } else {
            Err(SuperGitError::GitCommandFailed {
                args,
                status: result.status,
                stderr: result.stderr,
            })
        }
    }

    fn read_command(&self) -> Command {
        let mut command = self.base_command();
        // Read-side commands must not opportunistically refresh or lock the index.
        // Write actions use `write_command` so Git can take the locks it needs.
        command.env("GIT_OPTIONAL_LOCKS", "0");
        // Read commands accept arbitrary repository paths (inspect/status/wt
        // list/repo add). A hostile repo's core.fsmonitor is run as a command
        // even on read-only operations (GIT_OPTIONAL_LOCKS=0 does not suppress
        // it), so disable it: reads have no use for an fsmonitor. The -c override
        // wins over repo-local config. Write paths run against the plan-bound
        // repository and keep standard Git behavior; that limitation is
        // documented in docs/safety-model.md.
        command.args(["-c", "core.fsmonitor=false"]);
        command
    }

    fn write_command(&self) -> Command {
        self.base_command()
    }

    fn base_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        // Plans must bind to the repository selected by `git -C`, not ambient
        // Git environment inherited from an agent shell. Beyond the directory
        // family, ambient env can subvert safety in worse ways:
        // - GIT_CONFIG_COUNT/GIT_CONFIG_PARAMETERS inject arbitrary config such
        //   as core.fsmonitor / core.hooksPath, which is arbitrary command
        //   execution on plain read commands (GIT_OPTIONAL_LOCKS=0 does not
        //   suppress fsmonitor). Clearing GIT_CONFIG_COUNT also neutralizes any
        //   GIT_CONFIG_KEY_n/GIT_CONFIG_VALUE_n.
        // - GIT_NAMESPACE silently retargets ref reads and the history-edit
        //   compare-and-swap to a different namespace than the plan bound to.
        // - GIT_OBJECT_DIRECTORY/GIT_ALTERNATE_OBJECT_DIRECTORIES redirect where
        //   objects are read/written; GIT_EXTERNAL_DIFF/GIT_DIFF_OPTS run an
        //   external diff driver that would also poison the state fingerprint.
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_PREFIX",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
            "GIT_NAMESPACE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CEILING_DIRECTORIES",
            "GIT_EXTERNAL_DIFF",
            "GIT_DIFF_OPTS",
        ] {
            command.env_remove(key);
        }
        command
    }

    fn bytes_output_or_error(&self, args: Vec<String>, output: Output) -> Result<GitBytesOutput> {
        if output.status.success() {
            Ok(GitBytesOutput {
                stdout: output.stdout,
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        } else {
            Err(SuperGitError::GitCommandFailed {
                args,
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommandResult {
    pub success: bool,
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl GitCommandResult {
    fn from_output(output: Output) -> Self {
        Self {
            success: output.status.success(),
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        }
    }
}

fn collect_args<I, S>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect()
}

fn display_args(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::decode_git_path;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn decode_git_path_preserves_non_utf8_bytes() {
        // "/repo/caf<0xe9>" (latin-1 e-acute) with a trailing newline: the byte
        // is invalid UTF-8 and lossy conversion would replace it with U+FFFD,
        // producing a path that does not exist on disk.
        let path = decode_git_path(b"/repo/caf\xe9\n");
        assert_eq!(path.as_os_str().as_bytes(), b"/repo/caf\xe9");
    }

    #[test]
    fn decode_git_path_trims_single_trailing_newline() {
        let path = decode_git_path(b"/abs/repo\n");
        assert_eq!(path.as_os_str().as_bytes(), b"/abs/repo");
    }
}
