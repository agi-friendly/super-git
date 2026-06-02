mod args;
mod output;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use super_git_core::config::store::ConfigStore;
use super_git_core::git::command::Git;
use super_git_core::git::{status, worktree};

use crate::args::{Cli, Commands, RepoCommands, WorktreeCommands};
use crate::output::OutputMode;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // 기본은 JSON. `--human`을 줄 때만 사람용 출력으로 전환한다.
    let mode = if cli.human {
        OutputMode::Human
    } else {
        OutputMode::Json
    };

    match cli.command {
        Commands::Doctor => run_doctor(mode),
        Commands::Repo { command } => run_repo(mode, command),
        Commands::Status { path } => run_status(mode, path),
        Commands::Wt { command } => run_worktree(mode, command),
    }
}

fn run_doctor(mode: OutputMode) -> Result<()> {
    let git_version = Git::default()
        .version()
        .context("git is not installed or cannot run `git --version`")?;
    let store = ConfigStore::from_default_path().context("could not resolve config path")?;

    output::print_doctor(mode, &git_version, store.path())
}

fn run_repo(mode: OutputMode, command: RepoCommands) -> Result<()> {
    let store = ConfigStore::from_default_path().context("could not resolve config path")?;

    match command {
        RepoCommands::Add { path } => {
            let result = store
                .add_repository(&path)
                .with_context(|| format!("could not add repository {}", path.display()))?;
            output::print_repo_add(mode, &result.path, result.added)
        }
        RepoCommands::List => {
            let config = store.load().context("could not read config file")?;
            output::print_repo_list(mode, &config.repositories)
        }
    }
}

fn run_status(mode: OutputMode, path: Option<PathBuf>) -> Result<()> {
    let path = path_or_current_dir(path)?;
    let status = status::read_status(&path)
        .with_context(|| format!("could not read Git status for {}", path.display()))?;

    output::print_status(mode, &path, &status)
}

fn run_worktree(mode: OutputMode, command: WorktreeCommands) -> Result<()> {
    match command {
        WorktreeCommands::List { path } => {
            let path = path_or_current_dir(path)?;
            let worktrees = worktree::list_worktrees(&path)
                .with_context(|| format!("could not list worktrees for {}", path.display()))?;

            output::print_worktrees(mode, &worktrees)
        }
    }
}

fn path_or_current_dir(path: Option<PathBuf>) -> Result<PathBuf> {
    match path {
        Some(path) => Ok(path),
        None => std::env::current_dir().context("could not read current directory"),
    }
}
