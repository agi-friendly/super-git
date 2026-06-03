# Setup

이 문서는 다른 PC에서 `super-git` 저장소를 clone한 뒤 필요한 기본 설치 프로그램과 확인 명령을 정리한다.

## Short Answer

개발용으로 필요한 기본 프로그램은 다음과 같다.

- Git
- Rust toolchain
- Cargo
- rustfmt
- Clippy

`super-git`은 Git을 직접 재구현하지 않는다. 실행 시 설치된 시스템 `git` 명령을 호출하므로, Rust로 빌드할 때뿐 아니라 `super-git doctor`, `super-git status`, `super-git wt list` 같은 명령을 실행할 때도 `git`이 필요하다.

## Why Each Tool Is Needed

### Git

Git은 두 가지 이유로 필요하다.

첫째, 이 저장소를 clone하기 위해 필요하다.

```bash
git clone <repository-url>
```

둘째, `super-git` 자체가 내부에서 시스템 `git` 명령을 실행한다.

예를 들어 Stage 1에서는 다음 Git 명령들을 Rust에서 안전하게 감싼다.

```bash
git --version
git -C <path> status --porcelain=v1 --branch
git -C <path> worktree list --porcelain
git -C <path> rev-parse --is-inside-work-tree
```

따라서 `git --version`이 터미널에서 동작해야 한다.

### Rust Toolchain

Rust toolchain은 Rust 코드를 컴파일하기 위한 기본 도구 묶음이다.

일반적으로 다음 명령들이 포함된다.

```bash
rustc --version
cargo --version
```

`rustc`는 Rust 컴파일러이고, `cargo`는 Rust의 빌드 도구이자 패키지 매니저다.

### Cargo

Cargo는 Rust 프로젝트에서 가장 자주 쓰는 명령 도구다.

서버 개발 기준으로 비유하면 Gradle, Maven, npm, pnpm과 비슷한 위치에 있다.

이 프로젝트에서는 주로 다음 용도로 사용한다.

```bash
cargo build
cargo run -p super-git-cli -- doctor
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

### rustfmt

`rustfmt`는 Rust 코드 포맷터다.

코드 스타일을 자동으로 맞추기 위해 사용한다.

```bash
cargo fmt --all
cargo fmt --all --check
```

### Clippy

Clippy는 Rust 공식 linter다.

컴파일은 되지만 더 안전하거나 읽기 좋게 고칠 수 있는 코드, 헷갈리는 이름, 불필요한 패턴 등을 알려준다.

이 프로젝트에서는 경고를 에러처럼 다루기 위해 다음 명령을 사용한다.

```bash
cargo clippy --all-targets -- -D warnings
```

## macOS

권장 설치 방법은 `rustup`이지만, Homebrew Rust도 사용할 수 있다.

### Option A: rustup

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup component add clippy rustfmt
```

설치 후 새 터미널을 열고 확인한다.

```bash
git --version
rustc --version
cargo --version
cargo clippy --version
```

### Option B: Homebrew

```bash
brew install git rust
```

Homebrew Rust에는 보통 `rustc`, `cargo`, `rustfmt`, `clippy`가 함께 제공된다.

확인 명령은 동일하다.

```bash
git --version
rustc --version
cargo --version
cargo clippy --version
```

## Windows

Windows에서는 다음 설치가 기본이다.

- Git for Windows
- Rust toolchain via rustup
- Microsoft C++ Build Tools, if rustup or Cargo reports a missing linker

### Git for Windows

Git for Windows를 설치한 뒤 PowerShell 또는 Windows Terminal에서 확인한다.

```powershell
git --version
```

`super-git` 명령은 시스템 `git` 명령을 호출하므로, `git`이 PATH에 잡혀 있어야 한다.

### Rust via rustup

Rust는 rustup으로 설치하는 것을 권장한다.

설치 후 확인한다.

```powershell
rustc --version
cargo --version
cargo clippy --version
```

Clippy 또는 rustfmt가 없다면 다음을 실행한다.

```powershell
rustup component add clippy rustfmt
```

### Build Tools

Windows에서 Rust 빌드 중 linker 관련 오류가 나오면 Microsoft C++ Build Tools가 필요할 수 있다.

rustup 설치 과정에서 안내가 나오면 그 안내를 따른다.

## Linux

Linux에서는 배포판 패키지 매니저로 Git을 설치하고, Rust는 rustup으로 설치하는 것을 권장한다.

Ubuntu/Debian 예시는 다음과 같다.

```bash
sudo apt update
sudo apt install git build-essential
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup component add clippy rustfmt
```

확인한다.

```bash
git --version
rustc --version
cargo --version
cargo clippy --version
```

## Clone And Verify

다른 PC에서 처음 clone한 뒤에는 다음 순서로 확인한다.

```bash
git clone <repository-url>
cd super-git
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -p super-git-cli -- doctor
cargo run -p super-git-cli -- inspect
```

`doctor`는 기본적으로 JSON을 출력한다. 모든 명령의 출력은
`{ "ok": true, "data": {...} }`(성공) 또는 `{ "ok": false, "error": {...} }`(실패)
envelope를 따르며, 종료 코드로도 성공/실패를 구분할 수 있다(성공 0, 실패 1).

```json
{
  "ok": true,
  "data": {
    "arch": "aarch64",
    "config_path": "/Users/<name>/Library/Application Support/com.super-git.super-git/config.json",
    "git_version": "git version 2.54.0",
    "os": "macos"
  }
}
```

사람이 읽기 좋은 형태로 보려면 `--human`을 붙인다.

```bash
cargo run -p super-git-cli -- --human doctor
```

```text
super-git doctor
Git: OK (git version 2.54.0)
OS: macos aarch64
Config: /Users/<name>/Library/Application Support/com.super-git.super-git/config.json
```

`super-git inspect [path]`는 저장소의 현재 상태를 versioned safety snapshot으로
조회하는 AI-first 핵심 명령이다. 출력의 `repository`는 입력이 하위 디렉토리여도
항상 절대 worktree root로 정규화된다.

inspect JSON은 `summary`, `risk_hint`, `warnings`, `next`를 함께 제공한다.
`summary.execution_permission`은 `not_granted_by_inspect`, `risk_hint.scope`는
`current_state_only`, `next.execution_contract`는 `preview_required`다.
`next.allowed`는 바로 실행하라는 뜻이 아니라 preview 후보이고, `next.raw_git_allowed`는
항상 `false`다. action의 `reference_command`는 문서화용 참고 명령이지 실행 계약이 아니다.
`next.needs_human_review`는 `evaluated_actions` catalog 안에서만 해석한다.

## Runtime Config Location

`super-git repo add <path>`는 등록한 저장소 목록을 사용자별 config directory에 저장한다.

정확한 위치는 OS마다 다르며, 다음 명령으로 확인할 수 있다.

```bash
cargo run -p super-git-cli -- doctor
```

예상 위치는 대략 다음과 같다.

- macOS: `~/Library/Application Support/com.super-git.super-git/config.json`
- Windows: `%APPDATA%\super-git\super-git\config\config.json` 또는 유사한 사용자 config 경로
- Linux: `~/.config/super-git/config.json` 또는 유사한 XDG config 경로

정확한 경로는 `directories` crate가 OS별 표준 config directory를 기준으로 결정한다.

## Current Windows Status

Stage 1 코드는 Windows 지원을 염두에 두고 작성되어 있다.

- shell command string을 만들지 않고 `std::process::Command`로 실행한다.
- Git 인자는 배열로 전달한다.
- 경로와 Git 인자는 `OsStr`/`OsString` 기반으로 처리한다.
- config 경로는 `directories` crate로 OS별 표준 위치를 사용한다.

다만 현재까지 실제 검증은 macOS에서 수행했다. Windows에서는 아직 CI나 실기기 테스트를 추가하지 않았다.

Windows에서 우선 확인해야 할 명령은 다음과 같다.

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -p super-git-cli -- doctor
cargo run -p super-git-cli -- repo add .
cargo run -p super-git-cli -- status
cargo run -p super-git-cli -- wt list
```

## Optional Tools

아래 도구들은 필수는 아니지만 개발 경험을 좋게 만든다.

- VS Code 또는 RustRover
- rust-analyzer
- Git GUI 도구
- Tauri 관련 도구, Stage 5 이후 데스크톱 앱을 시작할 때 필요

Stage 1에서는 Tauri, Node.js, Svelte, Electron이 필요하지 않다.
