mod args;
mod output;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use supergit_core::config::store::ConfigStore;
use supergit_core::git::command::Git;
use supergit_core::git::{status, worktree};

use crate::args::{Cli, Commands, RepoCommands, WorktreeCommands};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Doctor => run_doctor(),
        Commands::Repo { command } => run_repo(command),
        Commands::Status { path } => run_status(path),
        Commands::Wt { command } => run_worktree(command),
    }
}

fn run_doctor() -> Result<()> {
    let git_version = Git::default()
        .version()
        .context("git is not installed or cannot run `git --version`")?;
    let store = ConfigStore::from_default_path().context("could not resolve config path")?;

    output::print_doctor(&git_version, store.path());
    Ok(())
}

fn run_repo(command: RepoCommands) -> Result<()> {
    let store = ConfigStore::from_default_path().context("could not resolve config path")?;

    match command {
        RepoCommands::Add { path } => {
            let result = store
                .add_repository(&path)
                .with_context(|| format!("could not add repository {}", path.display()))?;
            output::print_repo_add(&result.path, result.added);
        }
        RepoCommands::List => {
            let config = store.load().context("could not read config file")?;
            output::print_repo_list(&config.repositories);
        }
    }

    Ok(())
}

fn run_status(path: Option<PathBuf>) -> Result<()> {
    let path = path_or_current_dir(path)?;
    let status = status::read_status(&path)
        .with_context(|| format!("could not read Git status for {}", path.display()))?;

    output::print_status(&path, &status);
    Ok(())
}

fn run_worktree(command: WorktreeCommands) -> Result<()> {
    match command {
        WorktreeCommands::List { path } => {
            let path = path_or_current_dir(path)?;
            let worktrees = worktree::list_worktrees(&path)
                .with_context(|| format!("could not list worktrees for {}", path.display()))?;

            output::print_worktrees(&worktrees);
        }
    }

    Ok(())
}

fn path_or_current_dir(path: Option<PathBuf>) -> Result<PathBuf> {
    match path {
        Some(path) => Ok(path),
        None => std::env::current_dir().context("could not read current directory"),
    }
}
