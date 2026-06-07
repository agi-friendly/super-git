use std::path::PathBuf;

use clap::{ArgGroup, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "super-git")]
#[command(about = "A small CLI-first foundation for super-git")]
#[command(version)]
pub struct Cli {
    /// Render human-readable output instead of the default JSON.
    #[arg(long, global = true)]
    pub human: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Check the local environment.
    Doctor,

    /// Manage registered repositories.
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },

    /// Inspect super-git global configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Show Git status for a repository path or the current directory.
    Status { path: Option<PathBuf> },

    /// Inspect full repository state: HEAD and any in-progress operation.
    Inspect { path: Option<PathBuf> },

    /// Build a read-only plan for a future write action.
    Preview {
        #[command(subcommand)]
        command: PreviewCommands,
    },

    /// Execute a previously previewed plan after re-validation.
    Execute {
        /// Plan file to execute. Use '-' to read from stdin.
        #[arg(long)]
        plan: PathBuf,
    },

    /// Undo a write using a validated undo token.
    Undo {
        /// Undo token file. Use '-' to read from stdin.
        #[arg(long)]
        token: PathBuf,
    },

    /// Inspect Git worktrees.
    Wt {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum RepoCommands {
    /// Compatibility alias for `repo save <path>`.
    Add { path: PathBuf },

    /// Save a Git worktree family to the repository registry.
    Save { path: Option<PathBuf> },

    /// Remove a saved repository family from the registry only.
    Forget { target: String },

    /// List saved repository families.
    List,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    /// Show the resolved super-git app home and config file path.
    Path,

    /// Show the resolved config location and loaded config.
    Show,

    /// Validate the loaded config without writing it.
    Validate,

    /// Set worktree path/name template settings.
    #[command(group(
        ArgGroup::new("worktree_template")
            .required(true)
            .multiple(true)
            .args(["parent_template", "name_template", "ref_slug_algorithm"])
    ))]
    SetWorktreeTemplate {
        /// Template for the parent directory that contains linked worktrees.
        #[arg(long)]
        parent_template: Option<String>,

        /// Template for the linked worktree directory name.
        #[arg(long)]
        name_template: Option<String>,

        /// Slug algorithm used for ref names.
        #[arg(long)]
        ref_slug_algorithm: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PreviewCommands {
    /// Preview staging all current unstaged and untracked changes.
    StageChanges,

    /// Preview creating a linked worktree from a branch, tag, or commit.
    WorktreeCreate {
        /// Saved repository id/name/path selector. Defaults to the current Git worktree family.
        #[arg(long)]
        repo: Option<String>,

        /// Existing local branch, tag, commit, or remote-tracking branch to inspect.
        #[arg(long = "ref")]
        ref_name: String,
    },

    /// Preview removing an existing linked worktree.
    WorktreeRemove {
        /// Exact absolute linked worktree path to inspect for removal.
        #[arg(long)]
        worktree: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum WorktreeCommands {
    /// List worktrees for a repository path or the current directory.
    List { path: Option<PathBuf> },
}
