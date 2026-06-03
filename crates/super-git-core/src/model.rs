use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    pub path: PathBuf,
}

impl Repository {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusOutput {
    pub branch_header: Option<String>,
    pub entries: Vec<String>,
}

impl StatusOutput {
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }
}

pub const INSPECT_SCHEMA_VERSION: &str = "super-git.inspect.v0.2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
}

impl WorktreeInfo {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            head: None,
            branch: None,
            detached: false,
            bare: false,
        }
    }
}

/// 진행 중인 Git 작업. `.git` 내부의 상태 파일 존재 여부로 판별한다.
/// super-git의 핵심 가치: git의 숨은 상태머신을 명시적으로 드러낸다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    None,
    Merging,
    Rebasing,
    /// `git am` 세션 (mailbox 패치 적용 중).
    Applying,
    CherryPicking,
    Reverting,
    Bisecting,
}

/// HEAD가 가리키는 위치.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeadInfo {
    /// 현재 브랜치명. detached HEAD이면 None.
    pub branch: Option<String>,
    /// HEAD 커밋 SHA. 커밋이 아직 없는 새 저장소(unborn)이면 None.
    pub commit: Option<String>,
    /// HEAD가 브랜치가 아닌 커밋을 직접 가리키는 상태.
    pub detached: bool,
}

/// upstream(추적 브랜치) 대비 위치.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamComparisonBasis {
    LocalTrackingRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamComparisonStatus {
    Ok,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpstreamInfo {
    /// upstream 브랜치 이름 (예: "origin/main").
    pub name: String,
    /// HEAD가 upstream보다 앞선 커밋 수.
    pub ahead: u32,
    /// HEAD가 upstream보다 뒤처진 커밋 수.
    pub behind: u32,
    /// ahead/behind가 어떤 기준으로 계산됐는지. 지금은 fetch하지 않은 로컬 추적 ref 기준이다.
    pub comparison_basis: UpstreamComparisonBasis,
    /// 비교 명령이 성공했는지. 실패 시 ahead/behind 값은 신뢰하면 안 된다.
    pub comparison_status: UpstreamComparisonStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectWarning {
    pub code: String,
    pub severity: WarningSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectSummary {
    pub state: String,
    pub codes: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RiskFactor {
    pub code: String,
    pub level: RiskLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectRiskHint {
    pub level: RiskLevel,
    pub factors: Vec<RiskFactor>,
}

/// 워킹 트리 변경 요약. 상세 파일 목록은 `status` 명령이 담당하고,
/// 여기서는 AI가 다음 행동을 판단할 만큼의 카운트와 충돌 목록만 둔다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkingTree {
    pub clean: bool,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub conflict_count: u32,
    /// 충돌(unmerged) 파일 경로 목록. 해결 대상이라 목록으로 노출한다.
    pub conflicts: Vec<String>,
}

/// inspect가 제안하는 "다음에 할 수 있는 행동" 힌트.
/// 실행 엔진 계약이 아니라 AI가 판단할 수 있는 구조화된 hint다(나중 execute 라이프사이클의 씨앗).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextAction {
    /// 행동 종류 식별자 (예: "commit", "push", "rebase_abort").
    pub kind: String,
    /// 이 행동이 가능한 이유(현재 상태 근거).
    pub reason: String,
    /// 참고용 git 명령(canonical reference). 환경(EDITOR 등)에 따라 그대로 실행되지 않을 수도
    /// 있다 — execute 단계에서 환경에 맞게 보정한다. 없을 수도 있다(예: resolve_conflicts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// 되돌림 가능성 힌트("reversible" 등). 확실한 경우에만 채운다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextGuardrails {
    /// 안전한 preview 후보. raw Git 명령을 바로 실행해도 된다는 뜻은 아니다.
    pub allowed: Vec<NextAction>,
    /// 현재 상태에서 precondition이 맞지 않아 막아야 하는 행동.
    pub blocked: Vec<NextAction>,
    /// C4 preview/execute를 위해 예약된 bucket. 현재 inspect는 항상 빈 배열을 낸다.
    pub needs_human_review: Vec<NextAction>,
}

/// 현재 worktree가 worktree family에서 어떤 위치인지 나타낸다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeKind {
    Main,
    Linked,
    Bare,
    Unknown,
}

/// 현재 worktree의 family 내 위치 요약.
/// 전체 worktree 목록은 `wt list`가 담당하고, 여기서는 "나는 어디인가"만 요약한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeContext {
    pub kind: WorktreeKind,
    /// main worktree 경로(linked에서도 main을 가리킨다).
    /// bare-primary family에는 main worktree가 없으므로 None.
    pub main: Option<PathBuf>,
    /// family의 전체 worktree 수(main/bare 포함).
    pub family_count: u32,
    /// linked worktree 수(main/bare 제외).
    pub linked_count: u32,
}

/// 저장소의 현재 상태 스냅샷. `inspect`의 핵심 모델.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepoState {
    /// 저장소(워크트리) 루트의 절대경로. 입력이 하위 디렉토리여도 root로 정규화된다.
    pub root: PathBuf,
    /// 현재 worktree의 family 내 위치.
    pub worktree_context: WorktreeContext,
    pub head: HeadInfo,
    /// upstream 추적 브랜치 정보. 미설정/detached/unborn이면 None.
    pub upstream: Option<UpstreamInfo>,
    pub working_tree: WorkingTree,
    pub operation: Operation,
    /// 현재 상태에서 가능한 preview 후보와 막아야 하는 행동 힌트.
    pub next: NextGuardrails,
    pub warnings: Vec<InspectWarning>,
    pub summary: InspectSummary,
    pub risk_hint: InspectRiskHint,
}
