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
- `super-git inspect [path]` — self-describing versioned safety snapshot(summary/risk_hint/next guardrails 포함)
- `super-git wt list [path]`
- 설정 파일에 저장소 목록 저장
- worktree/status/state 파싱 + inspect 통합 테스트
- inspect는 읽기 전용으로 유지하고, `next.allowed`는 이후 preview 단계의 후보로만 해석
- inspect의 `reference_command`는 문서화용 참고값이며 raw Git 실행 허가가 아님

## Stage 2: Preview/Execute/Undo Foundation

- `preview` plan contract 추가
- `execute --plan <file|->` trust boundary 추가
- plan hash, state fingerprint, precondition mismatch 처리
- `undo_preview`와 execute 이후 `undo_token` 분리
- 첫 write action은 `stage_changes`로 제한
- execute는 plan의 `reference_commands`를 믿지 않고 내부 allowlist로 Git 명령 재생성

## Stage 3: Safe Worktree Create Dry-run

- worktree 생성 계획을 먼저 보여주는 preview 추가
- 브랜치 이름, worktree 경로, base branch 검증
- 충돌 가능성이 있는 경로와 브랜치 상태를 먼저 설명
- 실제 생성은 execute plan과 명시적 확인 옵션으로 제한

## Stage 4: Worktree Create/Remove

- 안전장치를 둔 worktree 생성
- 안전장치를 둔 worktree 제거
- dirty worktree, untracked file, branch 사용 여부 확인
- 삭제 전 명확한 preview 출력

## Stage 5: Multi-repository Dashboard Model

- 여러 저장소의 상태를 빠르게 요약하는 데이터 모델
- 등록 저장소별 branch/status/worktree 요약
- CLI에서 dashboard 형태 출력
- 나중 UI가 사용할 수 있는 안정적인 구조 만들기

## Stage 6: Desktop Prototype

- Tauri + Svelte 기반 데스크톱 앱 실험
- Core/CLI 기능을 감싸는 얇은 UI
- 저장소 목록, status, worktree 목록 표시
- 실제 Git 동작은 검증된 Core 기능을 통해 실행

## Stage 7+: Integrations and Advanced Git

- Windows Explorer integration
- macOS Finder integration
- Linux file manager integration
- plugin system 설계와 구현
- conflict helper
- patch create/apply workflow
- interactive rebase helper
- reflog, branch 정렬, repository browser 같은 고급 기능
