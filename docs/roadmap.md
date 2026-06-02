# super-git Roadmap

이 로드맵은 작게 실패하고 안전하게 배우기 위한 단계 계획이다. 각 단계는 이전 단계가 실제로 쓸 만해졌을 때만 넘어간다.

## Stage 1: CLI Skeleton (AI-first)

- Rust workspace 구성
- `super-git-core`와 `super-git-cli` 분리
- `super-git` CLI 바이너리 제공 (기본 출력은 JSON envelope, `--human` 지원)
- `super-git doctor`
- `super-git repo add <path>`
- `super-git repo list`
- `super-git status [path]`
- `super-git inspect [path]` — HEAD + 진행 중인 작업 상태머신 조회
- `super-git wt list [path]`
- 설정 파일에 저장소 목록 저장
- worktree/status/state 파싱 + inspect 통합 테스트

## Stage 2: Safe Worktree Create Dry-run

- worktree 생성 계획을 먼저 보여주는 dry-run 추가
- 브랜치 이름, worktree 경로, base branch 검증
- 충돌 가능성이 있는 경로와 브랜치 상태를 먼저 설명
- 실제 생성은 아직 하지 않거나 명시적 확인 옵션으로 제한

## Stage 3: Worktree Create/Remove

- 안전장치를 둔 worktree 생성
- 안전장치를 둔 worktree 제거
- dirty worktree, untracked file, branch 사용 여부 확인
- 삭제 전 명확한 preview 출력

## Stage 4: Multi-repository Dashboard Model

- 여러 저장소의 상태를 빠르게 요약하는 데이터 모델
- 등록 저장소별 branch/status/worktree 요약
- CLI에서 dashboard 형태 출력
- 나중 UI가 사용할 수 있는 안정적인 구조 만들기

## Stage 5: Desktop Prototype

- Tauri + Svelte 기반 데스크톱 앱 실험
- Core/CLI 기능을 감싸는 얇은 UI
- 저장소 목록, status, worktree 목록 표시
- 실제 Git 동작은 검증된 Core 기능을 통해 실행

## Stage 6+: Integrations and Advanced Git

- Windows Explorer integration
- macOS Finder integration
- Linux file manager integration
- plugin system 설계와 구현
- conflict helper
- patch create/apply workflow
- interactive rebase helper
- reflog, branch 정렬, repository browser 같은 고급 기능
