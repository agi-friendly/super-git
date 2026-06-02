# super-git Architecture

super-git은 CLI-first 구조로 시작한다. 첫 목표는 데스크톱 앱을 빨리 띄우는 것이 아니라, Git 작업을 안전하게 감싸는 작고 믿을 수 있는 기반을 만드는 것이다.

## Layers

### Core library

`super-git-core`는 Git 명령 실행, 저장소 검증, 설정 파일 읽기/쓰기, status/worktree 출력 파싱 같은 순수 기능을 담당한다.

Core는 UI를 알지 않는다. 나중에 CLI, 데스크톱 앱, 파일 탐색기 연동이 모두 같은 Core 기능을 호출할 수 있게 유지한다.

### CLI

`super-git-cli`는 `super-git` 바이너리를 제공한다. 모든 핵심 기능은 CLI에서 먼저 동작해야 한다.

Stage 1의 CLI는 다음 기능만 제공한다.

- `super-git doctor`
- `super-git repo add <path>`
- `super-git repo list`
- `super-git status [path]`
- `super-git wt list [path]`

### Desktop UI

데스크톱 UI는 나중 단계에서 얇은 레이어로 추가한다. 초기 후보는 Tauri + Svelte지만, Stage 1에서는 구현하지 않는다.

UI는 Git을 직접 실행하기보다 Core/CLI가 가진 검증된 동작을 감싸는 방향으로 둔다.

## Git Strategy

초기 버전은 Git 자체를 재구현하지 않는다. 설치된 시스템 `git` 명령을 `std::process::Command`로 안전하게 호출한다.

명령 인자는 문자열 하나로 합치지 않고 배열로 전달한다. Stage 1에서는 읽기 명령 위주로만 실행한다.

## Config

등록한 저장소 목록은 cross-platform config directory 아래의 JSON 파일에 저장한다.

현재 설정 파일은 `super-git-core`의 `ConfigStore`가 관리한다.

## Worktree Focus

worktree 관리는 super-git의 중요한 차별점이다. Stage 1에서는 목록 조회만 지원하고, 생성/삭제는 dry-run과 안전장치를 먼저 설계한 뒤 추가한다.

## Plugins

plugin system은 장기 목표다. 하지만 Stage 1에서는 구현하지 않는다.

초기에는 핵심 Git 흐름을 먼저 안정화하고, 어떤 기능이 정말 plugin으로 분리되어야 하는지 확인한 뒤 설계한다.
