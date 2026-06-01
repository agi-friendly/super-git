# ADR 0001: CLI First

## Status

Accepted

## Context

super-git의 장기 목표는 Fork, TortoiseGit, IntelliJ IDEA Git, VS Code Git Worktree Manager에서 좋았던 점을 모아 더 편한 Git 도구를 만드는 것이다.

하지만 처음부터 데스크톱 앱, 파일 탐색기 연동, plugin system, 고급 merge/rebase UI를 모두 만들면 실패 지점이 너무 많아진다.

Stage 1에서는 작고 안전한 기반이 필요하다.

## Decision

super-git은 CLI-first로 시작한다.

- Core library를 CLI와 분리한다.
- CLI는 `super-git` 바이너리로 제공한다.
- 데스크톱 UI는 Core/CLI 기능이 안정된 뒤 얇은 레이어로 추가한다.
- Git 기능은 먼저 시스템 `git` 명령을 감싸서 구현한다.
- plugin system은 핵심 workflow가 안정된 뒤로 미룬다.

## Why CLI First

CLI는 테스트하기 쉽고, 실패했을 때 원인을 추적하기 쉽다. 또한 데스크톱 UI 없이도 실제 Git 작업 흐름을 검증할 수 있다.

모든 핵심 기능이 CLI에서 동작하면, 나중에 데스크톱 앱은 그 기능을 감싸는 방식으로 안전하게 확장할 수 있다.

## Why Rust

Rust는 cross-platform CLI와 데스크톱 기반 코어를 만들기에 적합하다. 실행 파일 배포가 쉽고, 파일 경로와 프로세스 실행 같은 시스템 작업을 비교적 안전하게 다룰 수 있다.

## Why Not Desktop First

데스크톱 앱부터 시작하면 UI 상태, 빌드 설정, 패키징, OS별 동작 차이 때문에 Git workflow 자체를 검증하기 전에 복잡도가 커진다.

Stage 1에서는 Git 명령 wrapping, repository config, worktree 조회 같은 핵심 동작을 먼저 검증한다.

## Why Wrap System Git First

super-git은 Git을 재구현하지 않는다. 초기에는 설치된 시스템 `git`이 이미 잘하는 일을 안전하게 호출한다.

Rust에서는 `std::process::Command`로 shell command string을 만들지 않고 인자를 배열로 넘긴다. 이렇게 하면 경로와 공백이 섞인 입력도 더 안전하게 처리할 수 있다.

## Why Postpone Plugins

plugin system은 강력하지만 초기에 넣으면 구조가 빨리 무거워진다.

먼저 core workflow를 직접 구현해 보고, 어떤 기능이 정말 plugin으로 분리되어야 하는지 확인한 뒤 설계한다.
