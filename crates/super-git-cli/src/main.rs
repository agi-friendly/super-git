mod args;
mod output;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::error::ErrorKind;
use clap::Parser;
use super_git_core::config::store::{default_app_home, ConfigStore};
use super_git_core::git::command::Git;
use super_git_core::git::{execute, preview, state, status, undo, worktree};

use crate::args::{Cli, Commands, ConfigCommands, PreviewCommands, RepoCommands, WorktreeCommands};
use crate::output::OutputMode;

fn main() -> ExitCode {
    // 파싱 단계의 실패도 출력 계약을 지켜야 하므로 parse() 대신 try_parse()를 쓴다.
    // parse()는 에러 시 스스로 stderr에 평문을 찍고 종료해 JSON 계약을 우회한다.
    match Cli::try_parse() {
        Ok(cli) => {
            let mode = output_mode(cli.human);

            // 실패도 출력 계약을 지킨다: JSON 모드면 런타임 에러도 구조화해서 내보낸다.
            match run(mode, cli.command) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    output::print_error(mode, &err);
                    ExitCode::FAILURE
                }
            }
        }
        Err(err) => handle_parse_error(err),
    }
}

/// `--human` 여부로 출력 모드를 정한다. 기본은 JSON.
fn output_mode(human: bool) -> OutputMode {
    if human {
        OutputMode::Human
    } else {
        OutputMode::Json
    }
}

/// clap 파싱 결과를 출력 계약에 맞춰 처리한다.
/// 오직 명시적 `--help`/`--version`만 의도된 출력(exit 0)으로 두고, 서브커맨드/인자 누락을
/// 포함한 나머지 파싱 오류는 JSON/human 에러 계약(exit 1)을 따른다.
fn handle_parse_error(err: clap::Error) -> ExitCode {
    // `super-git`처럼 서브커맨드 없이 호출하면 clap은 help를 띄우며 끝낸다.
    // 이를 exit 0으로 두면 자동화 입장에서 "성공인데 stdout 비어 있음"이 되어 위험하므로
    // 정상 출력으로 취급하지 않는다.
    if matches!(
        err.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        let _ = err.print();
        return ExitCode::SUCCESS;
    }

    // 파싱이 실패해 Cli를 못 만들었으니 `--human` 여부만 직접 확인한다.
    let mode = if std::env::args().any(|arg| arg == "--human") {
        OutputMode::Human
    } else {
        OutputMode::Json
    };
    output::print_parse_error(mode, &err);
    ExitCode::FAILURE
}

fn run(mode: OutputMode, command: Commands) -> Result<()> {
    match command {
        Commands::Doctor => run_doctor(mode),
        Commands::Repo { command } => run_repo(mode, command),
        Commands::Config { command } => run_config(mode, command),
        Commands::Status { path } => run_status(mode, path),
        Commands::Inspect { path } => run_inspect(mode, path),
        Commands::Preview { command } => run_preview(mode, command),
        Commands::Execute { plan } => run_execute(mode, plan),
        Commands::Undo { token } => run_undo(mode, token),
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

fn run_config(mode: OutputMode, command: ConfigCommands) -> Result<()> {
    let app_home = default_app_home().context("could not resolve config path")?;
    let store = ConfigStore::new(app_home.config_file.clone());

    match command {
        ConfigCommands::Path => output::print_config_path(mode, &app_home),
        ConfigCommands::Show => {
            let config = store.load().context("could not read config file")?;
            output::print_config_show(mode, &app_home, &config)
        }
    }
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

fn run_inspect(mode: OutputMode, path: Option<PathBuf>) -> Result<()> {
    let path = path_or_current_dir(path)?;
    let state = state::read_state(&path)
        .with_context(|| format!("could not inspect {}", path.display()))?;

    output::print_inspect(mode, &state)
}

fn run_preview(mode: OutputMode, command: PreviewCommands) -> Result<()> {
    match command {
        PreviewCommands::StageChanges => {
            let path = std::env::current_dir().context("could not read current directory")?;
            let plan =
                preview::preview_stage_changes(&path).context("could not preview stage_changes")?;

            output::print_preview_plan(mode, &plan)
        }
    }
}

fn run_execute(mode: OutputMode, plan: PathBuf) -> Result<()> {
    let current_dir = std::env::current_dir().context("could not read current directory")?;
    let bytes = read_json_arg(&plan, "plan")?;

    let result =
        execute::execute_plan_bytes(&current_dir, &bytes).context("could not execute plan")?;
    output::print_execute_result(mode, &result)
}

fn run_undo(mode: OutputMode, token: PathBuf) -> Result<()> {
    let current_dir = std::env::current_dir().context("could not read current directory")?;
    let bytes = read_json_arg(&token, "token")?;

    let result = undo::undo_token_bytes(&current_dir, &bytes).context("could not undo token")?;
    output::print_undo_result(mode, &result)
}

fn read_json_arg(path: &PathBuf, label: &str) -> Result<Vec<u8>> {
    if path.as_os_str() == "-" {
        let mut bytes = Vec::new();
        let mut stdin = std::io::stdin();
        std::io::Read::read_to_end(&mut stdin, &mut bytes)
            .with_context(|| format!("could not read {label} from stdin"))?;
        return Ok(bytes);
    }

    std::fs::read(path).with_context(|| format!("could not read {label} {}", path.display()))
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
