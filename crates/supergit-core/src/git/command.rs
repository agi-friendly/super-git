use std::path::Path;
use std::process::{Command, Output};

use crate::{Result, SuperGitError};

#[derive(Debug, Clone)]
pub struct Git {
    program: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
}

impl Default for Git {
    fn default() -> Self {
        Self {
            program: "git".to_string(),
        }
    }
}

impl Git {
    pub fn version(&self) -> Result<String> {
        let output = self.run(&["--version"])?;
        Ok(output.stdout.trim().to_string())
    }

    pub fn run(&self, args: &[&str]) -> Result<GitOutput> {
        let output = Command::new(&self.program).args(args).output()?;
        self.output_or_error(args_to_strings(args), output)
    }

    pub fn run_in(&self, path: &Path, args: &[&str]) -> Result<GitOutput> {
        let output = Command::new(&self.program)
            .arg("-C")
            .arg(path)
            .args(args)
            .output()?;

        let mut display_args = vec!["-C".to_string(), path.display().to_string()];
        display_args.extend(args_to_strings(args));

        self.output_or_error(display_args, output)
    }

    pub fn try_run_in(&self, path: &Path, args: &[&str]) -> Result<GitCommandResult> {
        let output = Command::new(&self.program)
            .arg("-C")
            .arg(path)
            .args(args)
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

fn args_to_strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}
