# super-git

super-git은 여러 Git 도구를 사용하며 느꼈던 장점을 모아,
가볍고 기본에 충실한 Git 작업 도구를 만드는 실험 프로젝트다.

## Product Principles

- Git의 기본 동작을 신뢰성 있게 지원한다.
- 모든 핵심 기능은 CLI에서 사용할 수 있어야 한다.
- Desktop UI는 CLI/Core 기능을 감싸는 얇은 레이어로 둔다.
- Windows, macOS, Linux를 지원한다.
- 기본 앱은 가볍게 유지하고, 무거운 기능은 Plugin 형태로 분리한다.
- 여러 저장소를 하나의 프로그램에서 관리할 수 있어야 한다.
- Git worktree 작업을 가장 편하고 안전하게 수행할 수 있어야 한다.
- OS 파일 탐색기에서 바로 실행할 수 있어야 한다.

## Inspiration

- TortoiseGit: Windows Explorer integration, rebase UX, lightweight usage
- Fork: multi-repository management
- IntelliJ IDEA Git: conflict handling, patch, interactive rebase support
- VS Code Git Worktree Manager: worktree creation and workspace opening flow

## Initial Scope

The first version focuses on repository registration, repository status,
worktree listing, worktree creation, worktree removal, and opening worktrees
in external tools such as VS Code, IntelliJ IDEA, or terminal.

## Non-goals for Early Versions

- Reimplementing Git
- Replacing all features of existing Git GUI tools
- Building complex merge/rebase UI from the beginning
- Building a full plugin system before the core workflow is stable