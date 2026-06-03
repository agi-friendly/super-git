# super-git Architecture

super-git은 CLI-first 구조로 시작한다. 첫 목표는 데스크톱 앱을 빨리 띄우는 것이 아니라, Git 작업을 안전하게 감싸는 작고 믿을 수 있는 기반을 만드는 것이다.

## Layers

### Core library

`super-git-core`는 Git 명령 실행, 저장소 검증, 설정 파일 읽기/쓰기, status/worktree 출력 파싱 같은 순수 기능을 담당한다.

Core는 UI를 알지 않는다. 나중에 CLI, 데스크톱 앱, 파일 탐색기 연동이 모두 같은 Core 기능을 호출할 수 있게 유지한다.

### CLI

`super-git-cli`는 `super-git` 바이너리를 제공한다. 모든 핵심 기능은 CLI에서 먼저 동작해야 한다.

Stage 1의 CLI는 다음 기능만 제공한다. 출력은 AI-first로, 기본이 JSON envelope(성공 `{ ok, data }` / 실패 `{ ok, error }`)이며 `--human`으로 사람용 텍스트로 바꾼다.

- `super-git doctor`
- `super-git repo add <path>`
- `super-git repo list`
- `super-git status [path]`
- `super-git inspect [path]` — HEAD, worktree context, upstream, working tree, 진행 중 작업, summary/risk/next guardrail을 드러내는 versioned safety snapshot
- `super-git wt list [path]`

### Desktop UI

데스크톱 UI는 나중 단계에서 얇은 레이어로 추가한다. 초기 후보는 Tauri + Svelte지만, Stage 1에서는 구현하지 않는다.

UI는 Git을 직접 실행하기보다 Core/CLI가 가진 검증된 동작을 감싸는 방향으로 둔다.

## Git Strategy

초기 버전은 Git 자체를 재구현하지 않는다. 설치된 시스템 `git` 명령을 `std::process::Command`로 안전하게 호출한다.

명령 인자는 문자열 하나로 합치지 않고 배열로 전달한다. Stage 1에서는 읽기 명령 위주로만 실행한다.

`inspect`는 읽기 전용 계약이다. `summary.execution_permission`은
`not_granted_by_inspect`이고, `next.execution_contract`는 `preview_required`다.
`next.allowed`는 실행 허가가 아니라 preview 후보이며, `next.raw_git_allowed`는 항상
`false`다. action의 `reference_command`는 문서화용 참고값이고, 쓰기 작업의 실제
위험도와 실행 잠금은 이후 preview/execute 단계에서 별도로 계산한다.

## Config

등록한 저장소 목록은 cross-platform config directory 아래의 JSON 파일에 저장한다.

현재 설정 파일은 `super-git-core`의 `ConfigStore`가 관리한다.

## Worktree Focus

worktree 관리는 super-git의 중요한 차별점이다. Stage 1에서는 목록 조회만 지원하고, 생성/삭제는 dry-run과 안전장치를 먼저 설계한 뒤 추가한다.

## Plugins

plugin system은 장기 목표다. 하지만 Stage 1에서는 구현하지 않는다.

초기에는 핵심 Git 흐름을 먼저 안정화하고, 어떤 기능이 정말 plugin으로 분리되어야 하는지 확인한 뒤 설계한다.
