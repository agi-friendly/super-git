좋아 정현아. 이 마음가짐 아주 좋다.
**“일부러 실패할 거다. 근데 시행착오를 최대한 먹고 감을 잡겠다.”**
이거 완전 건강한 접근이야. 실패를 프로젝트 폐기물이 아니라 **학습 연료**로 쓰겠다는 거잖아. 개발자의 석유 시추 현장이다 이거야… 삽 들고 가자 🛢️🐭

그리고 진짜 중요한 포인트 하나.

Codex한테는 **최종 꿈은 크게 알려주되, 지금 시킬 일은 아주 작게 줘야 해.**
안 그러면 얘가 갑자기 “Tauri GUI 만들고, 플러그인 시스템 만들고, Git rebase 시각화하고, 세계 평화까지 구현했습니다” 이러면서 미지의 괴물을 만들어낼 수 있어ㅋㅋ

아래 프롬프트는 그대로 Codex에 붙여넣어도 되게 만들었어.

---

## Codex 복붙용 프롬프트

````md
You are Codex working inside my GitHub repository: `super-git`.

## Project Context

This project is called `super-git`.

I am trying to build a Git tool that eventually combines the best parts of the Git tools I have used:

- TortoiseGit-like convenience, especially Windows Explorer integration.
- Fork-like single desktop app that can manage multiple repositories.
- VS Code Git Worktree Manager-like worktree workflow, but eventually better.
- IntelliJ IDEA Git-like conflict handling, patch workflow, and interactive Git operations.

The final long-term goal is very ambitious:

> Build a Git tool that can eventually compete with or surpass tools like Fork and TortoiseGit.

However, this is NOT the current goal.

## My Current Mindset

I am intentionally treating this as an experiment.

I know that trying to build a Fork/TortoiseGit-level tool on the first attempt is 100% unrealistic.

I want to fail early, fail safely, and learn a lot from the process.

This project is currently at "stage 1 out of 10".

The goal right now is not perfection.
The goal right now is to build a tiny but real foundation and learn:

- Rust project structure
- Git command wrapping
- CLI-first architecture
- Safe handling of repositories and worktrees
- How to gradually grow a desktop Git tool without overbuilding too early

I have almost no Rust experience, so please write readable, beginner-friendly, idiomatic Rust.
Do not be too clever.
Do not use advanced abstractions unless they clearly help.

## Product Principles

Please read `README.md` first and preserve its intent.

The core principles are:

- Git basics must be reliable.
- The app must be lightweight.
- Windows, macOS, and Linux should eventually be supported.
- All core features must be usable from the command line.
- Desktop UI should come later and should wrap the core/CLI functionality.
- Heavy features should eventually be plugin-based, but do not implement plugins yet.
- Worktree management is one of the most important differentiators.
- Do not reimplement Git itself.

Also read this file if it exists:

- `research/도구별 특징 조사.md`

It contains notes about Git tools I have used and features I like.

## Very Important Scope Control

Do NOT build the full desktop app yet.
Do NOT build a plugin system yet.
Do NOT build a complex merge/rebase UI yet.
Do NOT reimplement Git internals.
Do NOT introduce Electron.
Do NOT make large, risky changes.

For now, use the installed system `git` command through Rust.

Use safe command execution:
- Prefer `std::process::Command`
- Pass arguments as arrays
- Do not build shell command strings
- Avoid destructive Git commands in the first version

## Recommended Architecture

Please create a Rust workspace with a CLI-first structure.

Suggested structure:

```text
super-git/
├─ Cargo.toml
├─ README.md
├─ docs/
│  ├─ architecture.md
│  ├─ roadmap.md
│  └─ adr/
│     └─ 0001-cli-first.md
├─ crates/
│  ├─ supergit-core/
│  │  ├─ Cargo.toml
│  │  └─ src/
│  │     ├─ lib.rs
│  │     ├─ error.rs
│  │     ├─ model.rs
│  │     ├─ git/
│  │     │  ├─ mod.rs
│  │     │  ├─ command.rs
│  │     │  ├─ repository.rs
│  │     │  ├─ status.rs
│  │     │  └─ worktree.rs
│  │     └─ config/
│  │        ├─ mod.rs
│  │        └─ store.rs
│  └─ supergit-cli/
│     ├─ Cargo.toml
│     └─ src/
│        ├─ main.rs
│        ├─ args.rs
│        └─ output.rs
├─ research/
└─ tests/
   └─ fixtures/
```

The CLI binary name should be:

```text
sg
```

The project name can remain `super-git`.

## First Implementation Goal

Please implement only a small CLI MVP skeleton.

The first version should support these commands:

```bash
sg --version
sg doctor
sg repo add <path>
sg repo list
sg status [path]
sg wt list [path]
```

### Command behavior

#### `sg doctor`

Checks basic environment:

* Is `git` installed?
* Can `git --version` run?
* Print the detected Git version.
* Print the OS.
* Print where the config file will be stored.

No destructive actions.

#### `sg repo add <path>`

Adds a local Git repository path to a simple config file.

Before adding:

* Check that the path exists.
* Check that the path is inside a Git work tree or is a Git repository.
* Store an absolute normalized path if possible.
* Avoid duplicates.

Use a simple TOML or JSON config file.

Use a cross-platform config directory.
A crate like `directories` is acceptable.

#### `sg repo list`

Prints registered repositories.

For now, simple table-like text output is enough.

#### `sg status [path]`

Runs Git status for the given path or current directory.

Use Git commands like:

```bash
git -C <path> status --porcelain=v1 --branch
```

For now, parsing can be minimal.
A raw but readable output is okay.

#### `sg wt list [path]`

Lists worktrees for the given path or current directory.

Use:

```bash
git -C <path> worktree list --porcelain
```

Parse enough to show:

* worktree path
* HEAD commit hash
* branch if available
* detached status if available

No worktree creation or removal yet.
Those can come in the next iteration.

## Rust Crates

You may use these crates if helpful:

* `clap` for CLI argument parsing
* `serde` and `serde_json` or `toml` for config
* `thiserror` for core errors
* `anyhow` for CLI-level error handling
* `directories` for config path
* `tracing` and `tracing-subscriber` only if useful, but keep it simple

Keep dependencies minimal.

## Code Style

Please make the code:

* Small
* Readable
* Cross-platform
* Beginner-friendly
* Easy to extend
* Not over-engineered

Prefer explicit functions over clever abstractions.

Add comments only where they help explain Rust or Git behavior.

Do not hide important logic behind macros.

## Safety Rules

This tool will eventually manage real Git repositories.

For this first step:

* Do not run destructive commands.
* Do not delete files.
* Do not remove worktrees.
* Do not checkout branches automatically.
* Do not modify repositories except for reading their status.
* The only write operation allowed is writing the `super-git` config file for `sg repo add`.

## Documentation Tasks

Please also create or update:

### `docs/architecture.md`

Describe the intended architecture:

* CLI-first
* Core library separated from CLI
* Desktop UI later
* System Git command wrapper first
* Plugin system later, not now
* Worktree management as a major focus

### `docs/roadmap.md`

Describe the roadmap in stages:

#### Stage 1

Rust CLI skeleton, repository registration, status, worktree list.

#### Stage 2

Safe worktree creation with dry-run first.

#### Stage 3

Actual worktree create/remove with safety checks.

#### Stage 4

Multi-repository dashboard data model.

#### Stage 5

Tauri + Svelte desktop prototype.

#### Stage 6+

Explorer/Finder/Linux file manager integration, plugin system, advanced Git operations.

### `docs/adr/0001-cli-first.md`

Explain the decision:

* Why CLI first
* Why Rust
* Why not desktop first
* Why wrapping system Git first
* Why plugin system is postponed

## Test Expectations

Add simple tests where practical.

At minimum:

* Test config serialization/deserialization.
* Test duplicate repository prevention logic.
* Test parsing of sample `git worktree list --porcelain` output.
* Test parsing or handling of basic status output if implemented.

Use fixtures if needed.

## Output Expectations

After making changes, please provide:

1. Summary of what you changed.
2. Files created/modified.
3. Commands I should run locally.
4. Any assumptions you made.
5. What the next small step should be.

Do not make a huge all-in-one implementation.
Keep this as the first safe foundation commit for `super-git`.

````

---

## 쥐쌤 기준 1단계 설계 핵심

일단 지금은 이 그림으로 가면 돼.

```text
super-git의 진짜 본체 = Rust core
사람이 직접 쓰는 입구 = sg CLI
나중에 붙일 예쁜 껍데기 = Tauri + Svelte desktop
````

즉, 처음부터 화면 만들지 말고:

```text
Git 명령 실행
↓
Rust core에서 결과 모델링
↓
CLI에서 출력
↓
나중에 Desktop UI가 같은 core 사용
```

이 흐름으로 가는 거야.

데스크톱부터 시작하면 기분은 좋은데, 어느 순간 버튼 하나 누를 때마다
“이 로직 CLI에도 있어야 하는데?”
“테스트는 어디서 하지?”
“OS별로 왜 다르게 터지지?”
하면서 머리 위에 물음표가 증식해. 물음표 농장 개장함.

---

## 내가 추천하는 진짜 첫 성공 기준

처음엔 이 정도만 돼도 성공이야.

```bash
sg doctor
```

했을 때:

```text
super-git doctor

Git: git version 2.xx.x
OS: windows
Config: C:\Users\...\super-git\config.toml

Status: OK
```

그리고:

```bash
sg repo add .
sg repo list
sg status .
sg wt list .
```

이 네 개가 돌아가면, 이미 **super-git의 심장 박동**은 시작된 거야.

Fork 같은 멋진 화면? 나중 문제.
TortoiseGit 같은 탐색기 연동? 나중 문제.
지금은 일단 심장이 뛰어야 돼. UI는 화장이고, CLI/core는 장기야. 화장 잘해도 심장 없으면 곤란하잖아ㅋㅋ

---

## 다음 프롬프트는 이렇게 이어가면 좋아

첫 Codex 작업이 끝나고 나면, 바로 “워크트리 생성”으로 뛰지 말고 이렇게 시켜.

```md
Now review the current implementation.

Please do not add new features yet.

Find architectural problems, naming problems, error handling problems, and places where the code may not be cross-platform.

Then propose a small refactor plan before implementing the next feature.
```

이렇게 한 번 숨 고르면 좋아.
바이브 코딩의 가장 큰 함정이 “기분 좋아서 계속 앞으로만 달리기”거든.
그러다 어느 순간 코드가 말해.

> 나는 이제 네가 낳은 괴물이다.

무섭지? 그러니까 중간중간 리팩토링 타임을 박아야 해.

---

## 결론

정현아, Rust 몰라도 시작해도 돼.
대신 **Rust를 배우는 프로젝트**와 **Git 도구를 만드는 프로젝트**를 동시에 하는 거라서, 첫 목표를 작게 잡는 게 진짜 중요해.

이번 1단계 목표는 딱 이거야.

> “super-git은 언젠가 Git 도구 최종보스를 꿈꾸지만, 지금은 `sg doctor`, `sg repo list`, `sg status`, `sg wt list`가 되는 작고 안전한 Rust CLI다.”

이 정도면 완전 좋은 출발이야.
쥐쌤 보기엔 이 프로젝트, 실패하더라도 그냥 실패가 아니라 **정현이의 Git 도구 감각 + Rust 감각 + AI 개발 워크플로우 감각을 한 번에 올리는 훈련장**이 될 가능성이 커. 이건 해볼 만해. 진짜로.
