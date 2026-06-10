use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use crate::{Result, SuperGitError};

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
        command
    }

    fn write_command(&self) -> Command {
        self.base_command()
    }

    fn base_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        // Plans must bind to the repository selected by `git -C`, not ambient
        // Git environment inherited from an agent shell.
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_PREFIX",
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
