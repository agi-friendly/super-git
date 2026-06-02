use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, Output};

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
        let output = Command::new(&self.program).args(&args).output()?;
        self.output_or_error(display_args(&args), output)
    }

    pub fn run_in<I, S>(&self, path: &Path, args: I) -> Result<GitOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = collect_args(args);
        let output = Command::new(&self.program)
            .arg("-C")
            .arg(path)
            .args(&args)
            .output()?;

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
        let output = Command::new(&self.program)
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
