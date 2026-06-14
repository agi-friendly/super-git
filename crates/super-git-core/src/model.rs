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

pub const INSPECT_SCHEMA_VERSION: &str = "super-git.inspect.v0.3";
pub const PLAN_SCHEMA_VERSION: &str = "super-git.plan.v0.1";
pub const WORKTREE_PLAN_SCHEMA_VERSION: &str = "super-git.plan.v0.2";
pub const DESTRUCTIVE_PREVIEW_PLAN_SCHEMA_VERSION: &str = "super-git.plan.v0.3";
// v0.5: drop 지원이 plan_id projection에 prediction과 drop precondition들을
// 더하면서 v0.4 hash 계약이 바뀌었다. 구조 자체는 serde default로 하위호환이지만
// projection이 달라졌으므로, 옛 v0.4 plan은 plan_id mismatch가 아니라 명확한
// unsupported_schema_version으로 거부되도록 버전을 올린다.
pub const HISTORY_EDIT_PLAN_SCHEMA_VERSION: &str = "super-git.plan.v0.5";
pub const HISTORY_EDIT_INSTRUCTIONS_SCHEMA_VERSION: &str = "super-git.instructions.v0.1";
pub const CONFIRMATION_SCHEMA_VERSION: &str = "super-git.confirmation.v0.1";
pub const FINGERPRINT_SCHEMA_VERSION: &str = "super-git.fingerprint.v0.1";
pub const EXECUTE_SCHEMA_VERSION: &str = "super-git.execute.v0.2";
pub const UNDO_TOKEN_SCHEMA_VERSION: &str = "super-git.undo.v0.1";
pub const UNDO_REGISTRY_SCHEMA_VERSION: &str = "super-git.undo-registry.v0.1";
pub const UNDO_RESULT_SCHEMA_VERSION: &str = "super-git.undo-result.v0.1";
pub const WORKTREE_EXECUTION_RECORD_SCHEMA_VERSION: &str = "super-git.worktree-execution.v0.1";
pub const WORKTREE_REMOVE_EXECUTION_RECORD_SCHEMA_VERSION: &str =
    "super-git.worktree-remove-execution.v0.1";
pub const CONFLICT_PREDICTION_SCHEMA_VERSION: &str = "super-git.conflict-prediction.v0.1";
pub const HISTORY_EDIT_UNDO_TOKEN_SCHEMA_VERSION: &str = "super-git.history-edit-undo.v0.1";
pub const HISTORY_EDIT_EXECUTION_RECORD_SCHEMA_VERSION: &str =
    "super-git.history-edit-execution.v0.1";

pub const EVALUATED_INSPECT_ACTIONS: &[&str] = &[
    "stage_changes",
    "commit",
    "push",
    "pull",
    "integrate_diverged",
    "resolve_conflicts",
    "continue_operation",
    "merge_continue",
    "merge_abort",
    "rebase_continue",
    "rebase_skip",
    "rebase_abort",
    "am_continue",
    "am_skip",
    "am_abort",
    "cherry_pick_continue",
    "cherry_pick_skip",
    "cherry_pick_abort",
    "revert_continue",
    "revert_skip",
    "revert_abort",
    "bisect_reset",
    "worktree_create",
    "history_edit",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: bool,
    pub prunable: bool,
}

impl WorktreeInfo {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            head: None,
            branch: None,
            detached: false,
            bare: false,
            locked: false,
            prunable: false,
        }
    }
}

/// 진행 중인 Git 작업. `.git` 내부의 상태 파일 존재 여부로 판별한다.
/// super-git의 핵심 가치: git의 숨은 상태머신을 명시적으로 드러낸다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Merging => "merging",
            Self::Rebasing => "rebasing",
            Self::Applying => "applying",
            Self::CherryPicking => "cherry-picking",
            Self::Reverting => "reverting",
            Self::Bisecting => "bisecting",
        }
    }
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
    pub state_scope: String,
    pub execution_permission: String,
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
    pub scope: String,
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
    /// 참고용 명령(canonical reference) — git 또는 super-git preview 진입점.
    /// 실행 허가가 아니라 문서화용 예시이며, `<ref>` 같은 placeholder는
    /// 그대로 실행할 수 없는 형태로 둬서 오해를 막는다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_command: Option<Vec<String>>,
    /// 되돌림 가능성 힌트("reversible" 등). 확실한 경우에만 채운다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextGuardrails {
    pub scope: String,
    pub execution_contract: String,
    pub allowed_semantics: String,
    pub blocked_semantics: String,
    pub needs_human_review_scope: String,
    pub raw_git_allowed: bool,
    pub evaluated_actions: Vec<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewPlan {
    pub schema_version: String,
    pub plan_id: String,
    pub action: PreviewAction,
    pub repository: PathBuf,
    pub state_fingerprint: StateFingerprint,
    pub preconditions: Vec<PreviewPrecondition>,
    pub risk: ActionRisk,
    pub effects: Vec<String>,
    pub reference_commands: Vec<Vec<String>>,
    pub undo_strategy: UndoStrategy,
    pub undo_preview: UndoPreview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewAction {
    pub kind: String,
    pub scope: String,
    pub resolved_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCreatePlan {
    pub schema_version: String,
    pub plan_id: String,
    pub action: WorktreeCreateAction,
    pub repository: WorktreeCreateRepository,
    pub config_used: WorktreeCreateConfigUsed,
    pub source_ref: WorktreeSourceRef,
    pub ref_policy: WorktreeRefPolicy,
    pub target: WorktreeCreateTarget,
    pub family_snapshot: WorktreeFamilySnapshot,
    pub preconditions: Vec<WorktreeCreatePrecondition>,
    pub execution: WorktreeCreateExecution,
    pub risk: ActionRisk,
    pub effects: Vec<String>,
    pub reference_commands: WorktreeReferenceCommands,
    pub undo_strategy: WorktreeCreateUndoStrategy,
    pub undo_preview: WorktreeCreateUndoPreview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCreateAction {
    pub kind: String,
    pub options: WorktreeCreateOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCreateOptions {
    pub repo_selector: Option<String>,
    #[serde(rename = "ref")]
    pub ref_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCreateRepository {
    pub family_id: String,
    pub kind: String,
    pub git_common_dir: PathBuf,
    pub main_worktree: Option<PathBuf>,
    pub selected_from: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCreateConfigUsed {
    pub source: String,
    pub config_home_source: String,
    pub config_fingerprint: String,
    pub worktree_template: WorktreeTemplateConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeTemplateConfig {
    pub parent_template: String,
    pub name_template: String,
    pub ref_slug_algorithm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeSourceRef {
    pub input: String,
    pub kind: String,
    pub full_ref: Option<String>,
    pub resolved_commit: Option<String>,
    pub supported_for_execute: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRefPolicy {
    pub mode: String,
    pub will_create_branch: bool,
    pub will_detach_head: bool,
    pub will_track_upstream: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCreateTarget {
    pub path: PathBuf,
    pub parent: PathBuf,
    pub name: String,
    pub ref_slug: String,
    pub variables: WorktreeTemplateVariablesView,
    pub exists: bool,
    pub parent_exists: bool,
    pub parent_is_directory: bool,
    pub parent_is_symlink: bool,
    pub parent_creation: WorktreeParentCreationView,
    pub inside_git_dir: bool,
    pub inside_existing_worktree: bool,
    pub case_insensitive_collision: bool,
    pub reserved_name_collision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeTemplateVariablesView {
    pub main_path: PathBuf,
    pub repo_name: String,
    pub ref_slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeParentCreationView {
    pub allowed: bool,
    pub will_create: bool,
    pub removable_by_undo_if_empty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeFamilySnapshot {
    pub fingerprint: String,
    pub worktrees: Vec<WorktreeSnapshotEntry>,
    pub branch_occupancy: Vec<BranchOccupancy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeSnapshotEntry {
    pub path: PathBuf,
    pub kind: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchOccupancy {
    pub branch: String,
    pub worktree_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCreatePrecondition {
    pub code: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCreateExecution {
    pub status: String,
    pub super_git_execute_required: bool,
    pub raw_git_allowed: bool,
    pub suggested_super_git_command: Option<Vec<String>>,
    pub blocked_reasons: Vec<WorktreeBlockedReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeBlockedReason {
    pub code: String,
    pub severity: String,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeReferenceCommands {
    pub semantics: String,
    pub never_execute_directly: bool,
    pub commands: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCreateUndoStrategy {
    pub kind: String,
    pub deletes_branch: bool,
    pub deletes_history: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCreateUndoPreview {
    pub kind: String,
    pub available_after_execute: bool,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemovePlan {
    pub schema_version: String,
    pub plan_id: String,
    pub action: WorktreeRemoveAction,
    pub repository: WorktreeRemoveRepository,
    pub target: WorktreeRemoveTarget,
    pub target_state: WorktreeRemoveTargetState,
    pub preconditions: Vec<WorktreeRemovePrecondition>,
    pub execution: DestructivePreviewExecution,
    pub risk: ActionRisk,
    pub confirmation: PreviewConfirmation,
    pub effects: Vec<String>,
    pub limitations: Vec<String>,
    pub reference_commands: WorktreeReferenceCommands,
    pub undo_strategy: UnavailableUndoStrategy,
    pub recovery_hints: Vec<RecoveryHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoveAction {
    pub kind: String,
    pub options: WorktreeRemoveOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoveOptions {
    pub worktree: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoveRepository {
    pub family_id: String,
    pub git_common_dir: PathBuf,
    pub main_worktree: Option<PathBuf>,
    pub selected_from: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoveTarget {
    pub input_path: PathBuf,
    pub canonical_path: PathBuf,
    pub worktree_list_path: PathBuf,
    pub kind: String,
    pub worktree_git_dir: Option<PathBuf>,
    pub git_common_dir: Option<PathBuf>,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
    pub is_current_worktree: bool,
    pub has_submodules: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoveTargetState {
    pub operation: Operation,
    pub working_tree: WorktreeRemoveWorkingTree,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoveWorkingTree {
    pub clean: bool,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub ignored: u32,
    pub conflict_count: u32,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemovePrecondition {
    pub code: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestructivePreviewExecution {
    pub status: String,
    pub execute_supported: bool,
    pub future_execute_eligibility: String,
    pub raw_git_allowed: bool,
    pub suggested_super_git_command: Option<Vec<String>>,
    pub blocked_reasons: Vec<WorktreeBlockedReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewConfirmation {
    pub required_before_execute: bool,
    pub reason_codes: Vec<String>,
    pub human_prompt: String,
    /// The exact phrase the confirmation artifact's acknowledgement must carry.
    /// Advisory (excluded from plan_id, like human_prompt): execute re-derives
    /// the phrase from plan-bound fields, so tampering here cannot relax the
    /// check -- it only saves agents from reconstructing the phrase by trial
    /// and error. `default` keeps plans from older binaries deserializable.
    #[serde(default)]
    pub required_phrase: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnavailableUndoStrategy {
    pub kind: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryHint {
    pub kind: String,
    pub description: String,
    pub reference_command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoveConfirmation {
    pub schema_version: String,
    pub kind: Option<String>,
    pub action: Option<String>,
    pub plan_schema_version: Option<String>,
    pub plan_id: Option<String>,
    pub target: Option<WorktreeRemoveConfirmationTarget>,
    pub acknowledged_reason_codes: Option<Vec<String>>,
    pub acknowledged_undo_strategy: Option<String>,
    pub acknowledgement: Option<WorktreeRemoveAcknowledgement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoveConfirmationTarget {
    pub worktree_list_path: Option<PathBuf>,
    pub git_common_dir: Option<PathBuf>,
    pub head: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoveAcknowledgement {
    pub method: Option<String>,
    pub phrase: Option<String>,
}

/// `super-git.confirmation.v0.1` 아티팩트의 history_edit 변형.
/// published 히스토리 재작성 실행 권한을 명시적으로 증명한다.
/// target은 worktree_remove와 달리 분기 ref/tip 신원을 담는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditConfirmation {
    pub schema_version: String,
    pub kind: Option<String>,
    pub action: Option<String>,
    pub plan_schema_version: Option<String>,
    pub plan_id: Option<String>,
    pub target: Option<HistoryEditConfirmationTarget>,
    pub acknowledged_reason_codes: Option<Vec<String>>,
    pub acknowledged_undo_strategy: Option<String>,
    pub acknowledgement: Option<WorktreeRemoveAcknowledgement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditConfirmationTarget {
    pub branch_ref: Option<String>,
    pub git_common_dir: Option<PathBuf>,
    pub tip_commit: Option<String>,
}

/// `super-git.plan.v0.5` 히스토리 편집 계획.
/// pick/reword/squash/fixup는 트리를 보존해 분기 ref만 이동하고, drop은 patch를
/// 최종 history에서 제거한다(prediction이 plan_id에 바인딩된다). reorder는
/// prediction과 instruction order로 바인딩되고, reorder 요약은 advisory다.
/// instructions/result_summary는 survey 모드에서 null로 명시된다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPlan {
    pub schema_version: String,
    pub plan_id: String,
    pub action: HistoryEditAction,
    pub repository: HistoryEditPlanRepository,
    pub branch: Option<HistoryEditPlanBranch>,
    pub range: HistoryEditPlanRange,
    pub published_scan: HistoryEditPublishedScan,
    pub instructions: Option<HistoryEditPlanInstructions>,
    /// Filled on survey plans (no instructions supplied) so the agent can edit
    /// and resubmit it. Advisory: excluded from plan_id; survey plans are not
    /// executable, so the template carries no write authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions_template: Option<HistoryEditInstructionsTemplate>,
    pub result_summary: Option<HistoryEditResultSummaryView>,
    /// Reorder 전용 agent-facing summary. Advisory only: the authoritative
    /// order is `instructions.items`, and the replay prediction is plan-id bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reorder: Option<HistoryEditReorderAdvisory>,
    /// drop/reorder replay plan에 채워지는 예측 증거.
    /// advisory가 아니라 plan-binding이다: plan_id projection에 포함되고,
    /// `final_tree`는 replay-backed execute의 post-verify 오라클이 된다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prediction: Option<HistoryEditPrediction>,
    pub preconditions: Vec<HistoryEditPrecondition>,
    pub execution: HistoryEditExecution,
    pub risk: ActionRisk,
    /// published 범위를 실행하려면 별도 confirmation 아티팩트가 필요할 때만 채운다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<PreviewConfirmation>,
    pub warnings: Vec<HistoryEditPlanWarning>,
    pub effects: Vec<String>,
    pub limitations: Vec<String>,
    pub reference_commands: WorktreeReferenceCommands,
    pub undo_strategy: HistoryEditUndoStrategy,
    pub undo_preview: HistoryEditUndoPreview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditAction {
    pub kind: String,
    pub options: HistoryEditOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditOptions {
    pub base: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPlanRepository {
    pub family_id: String,
    pub git_common_dir: PathBuf,
    pub worktree_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPlanBranch {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub short_name: String,
    pub tip_commit: String,
    pub checked_out_at: PathBuf,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPlanRange {
    pub base_input: String,
    pub base_commit: String,
    pub base_is_ancestor_of_head: bool,
    pub order: String,
    pub commit_count: usize,
    pub commits: Vec<HistoryEditPlanCommit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPlanCommit {
    pub commit: String,
    pub subject: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub author_date: String,
    pub published: bool,
    pub signed: bool,
    pub is_merge: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPublishedScan {
    pub basis: String,
    pub published_commits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPlanInstructions {
    pub schema_version: String,
    pub order: String,
    pub items: Vec<HistoryEditPlanInstructionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPlanInstructionItem {
    pub commit: String,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A ready-to-edit `super-git.instructions.v0.1` document carried by survey
/// plans: every range commit prefilled as `pick`, in the exact shape
/// `preview history-edit --instructions` accepts. Agents copy it, change ops
/// and messages, and feed it back -- instead of reconstructing the schema from
/// docs or error breadcrumbs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditInstructionsTemplate {
    pub schema_version: String,
    pub action: String,
    pub base: String,
    pub items: Vec<HistoryEditPlanInstructionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditResultSummaryView {
    pub commits_before: u32,
    pub commits_after: u32,
    pub messages_changed: u32,
    pub commits_folded: u32,
    /// drop 도입 전의 plan과도 호환되도록 default(0).
    #[serde(default)]
    pub commits_dropped: u32,
    pub final_tree_unchanged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditReorderAdvisory {
    pub commits_reordered: u32,
    pub old_order: Vec<String>,
    pub new_order: Vec<String>,
}

/// history_edit replay 예측 (C8-drop / C8-reorder 계약).
/// C9 rebase-chain과 같은 per-step shape를 쓰되 plan에 바인딩된다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPrediction {
    /// "kept_commit_replay" | "reordered_commit_replay".
    pub kind: String,
    /// "clean" | "conflicted".
    pub status: String,
    /// 최종 history에서 patch가 제거되는 커밋들(oldest first).
    pub dropped_commits: Vec<String>,
    /// 전 step clean일 때 예측된 최종 트리 — execute post-verify 오라클.
    /// 이 값이 없는 tree-changing plan은 실행될 수 없다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_tree: Option<String>,
    /// kept 커밋별 replay 예측(oldest first). 첫 충돌에서 멈춘다.
    pub steps: Vec<HistoryEditPredictionStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPredictionStep {
    pub commit: String,
    /// 3-way base로 쓴 이 커밋의 원래 parent(드랍된 커밋일 수 있다).
    pub parent: String,
    /// "clean" | "conflicted".
    pub status: String,
    pub merged_tree: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicted_files: Vec<PredictedConflictFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPrecondition {
    pub code: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditExecution {
    pub status: String,
    pub execute_supported: bool,
    pub requires_confirmation_artifact: bool,
    pub raw_git_allowed: bool,
    pub suggested_super_git_command: Option<Vec<String>>,
    pub blocked_reasons: Vec<HistoryEditBlockedReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditBlockedReason {
    pub code: String,
    pub severity: String,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditPlanWarning {
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditUndoStrategy {
    pub kind: String,
    pub deletes_branch: bool,
    pub deletes_history: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditUndoPreview {
    pub kind: String,
    pub available_after_execute: bool,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateFingerprint {
    pub schema_version: String,
    pub repository: PathBuf,
    pub head_commit: Option<String>,
    pub operation: Operation,
    pub status_porcelain_v1_z_sha256: String,
    pub staged_diff_sha256: String,
    pub unstaged_diff_sha256: String,
    pub untracked_content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewPrecondition {
    pub code: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRisk {
    pub severity: String,
    pub reversibility: String,
    pub requires_human_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoStrategy {
    pub kind: String,
    pub requires_index_snapshot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoPreview {
    pub kind: String,
    pub available_after_execute: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteResult {
    pub schema_version: String,
    pub plan_id: String,
    pub action: String,
    pub repository: PathBuf,
    pub executed: bool,
    pub effects: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_token: Option<ExecuteUndoToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExecuteUndoToken {
    Index(Box<UndoToken>),
    Worktree(Box<WorktreeUndoToken>),
    HistoryEdit(Box<HistoryEditUndoToken>),
}

impl ExecuteUndoToken {
    pub fn kind(&self) -> &str {
        match self {
            Self::Index(token) => &token.kind,
            Self::Worktree(token) => &token.kind,
            Self::HistoryEdit(token) => &token.kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoToken {
    pub schema_version: String,
    pub kind: String,
    pub repository: PathBuf,
    pub action: String,
    pub plan_id: String,
    pub target_paths: Vec<String>,
    pub index_snapshot_path: PathBuf,
    pub pre_index_existed: bool,
    pub pre_index_sha256: String,
    pub post_index_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeUndoToken {
    pub schema_version: String,
    pub kind: String,
    pub repository: PathBuf,
    pub action: String,
    pub plan_id: String,
    pub target_path: PathBuf,
    pub target_head: String,
    pub target_branch: Option<String>,
    pub git_common_dir: PathBuf,
    pub family_id: String,
    pub source_ref: WorktreeSourceRef,
    pub ref_policy: WorktreeRefPolicy,
    pub created_parent: Option<PathBuf>,
    pub execution_record_path: PathBuf,
    pub deletes_branch: bool,
    pub deletes_history: bool,
}

/// 히스토리 편집 undo 토큰. 분기 ref를 이전 tip으로 되돌리는 것만 보장한다.
/// 워킹 트리/인덱스/다른 ref는 절대 건드리지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditUndoToken {
    pub schema_version: String,
    pub kind: String,
    pub repository: PathBuf,
    pub action: String,
    pub plan_id: String,
    pub branch_ref: String,
    pub previous_tip: String,
    pub new_tip: String,
    pub git_common_dir: PathBuf,
    pub family_id: String,
    pub execution_record_path: PathBuf,
    pub deletes_branch: bool,
    pub deletes_history: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditExecutionRecord {
    pub schema_version: String,
    pub status: String,
    pub action: String,
    pub plan_id: String,
    pub repository: HistoryEditPlanRepository,
    pub branch_ref: String,
    pub previous_tip: String,
    pub new_tip: String,
    pub final_tree: String,
    pub commits_before: u32,
    pub commits_after: u32,
    pub undo_token: Option<HistoryEditUndoToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeExecutionRecord {
    pub schema_version: String,
    pub status: String,
    pub action: String,
    pub plan_id: String,
    pub repository: WorktreeCreateRepository,
    pub target_path: PathBuf,
    pub source_ref: WorktreeSourceRef,
    pub expected_head: String,
    pub expected_branch: Option<String>,
    pub created_parent: Option<PathBuf>,
    pub undo_token: Option<WorktreeUndoToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoveExecutionRecord {
    pub schema_version: String,
    pub status: String,
    pub action: String,
    pub plan_id: String,
    pub repository: WorktreeRemoveRepository,
    pub target: WorktreeRemoveTarget,
    pub target_state: WorktreeRemoveTargetState,
    pub confirmation_reason_codes: Vec<String>,
    pub automatic_undo_available: bool,
    pub undo_strategy: UnavailableUndoStrategy,
    pub trusted_git_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoRegistryRecord {
    pub schema_version: String,
    pub token_sha256: String,
    pub undo_token: UndoToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UndoResult {
    pub schema_version: String,
    pub action: String,
    pub repository: PathBuf,
    pub plan_id: String,
    pub undone: bool,
    pub effects: Vec<String>,
}

/// Stage 7 충돌 예측 결과. 계약: docs/internal/plans/2026-06-12-c9-0-conflict-prediction-contract.md
/// plan이 아니라 read 결과다: plan_id도, execute/undo 대상도 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictPrediction {
    pub schema_version: String,
    /// 지금은 "merge"만. rebase-step 재사용 시 같은 shape에 kind만 늘어난다.
    pub prediction_kind: String,
    pub repository: PathBuf,
    pub inputs: ConflictPredictionInputs,
    pub prediction: ConflictPredictionOutcome,
    /// 과장 방지용 고정 문구(예: merge 예측 ≠ rebase 트랜스크립트). advisory.
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictPredictionInputs {
    pub ours: ResolvedRev,
    pub theirs: ResolvedRev,
    /// `git merge-base`의 best ancestor 하나. merge-tree 내부 recursive 병합은
    /// 여러 base를 합칠 수 있으므로 informational이다.
    pub merge_base: Option<String>,
}

/// 호출자가 준 rev 표기와 그것이 풀린 commit oid를 함께 보존한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedRev {
    pub rev: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictPredictionOutcome {
    /// "clean" | "conflicted". 충돌 예측은 성공한 예측이지 에러가 아니다.
    pub status: String,
    /// 병합 결과 트리 oid. conflicted면 충돌 마커가 들어간 트리다.
    pub merged_tree: String,
    pub conflicted_files: Vec<PredictedConflictFile>,
    pub notes: Vec<ConflictPredictionNote>,
}

/// 한 경로의 충돌을 index stage 존재 여부로 기계 판별 가능하게 묶는다.
/// stage 1=base, 2=ours, 3=theirs. 빠진 stage가 충돌 모양을 말해준다
/// (예: modify/delete는 한쪽 stage가 없다). 소비자는 메시지가 아니라
/// stage 존재 여부로 분기해야 한다.
/// Deserialize는 history_edit plan에 임베드될 때의 round-trip용이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredictedConflictFile {
    pub path: String,
    pub stages: Vec<PredictedConflictStage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredictedConflictStage {
    pub stage: u8,
    pub mode: String,
    pub object: String,
}

/// merge-tree informational stanza. kind 토큰("CONFLICT (contents)" 등)과
/// paths는 로케일과 무관하게 안정적이고, message는 번역되는 자유 텍스트라
/// 표시 전용이다. 어떤 코드도 message를 파싱/해시/분기에 쓰면 안 된다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictPredictionNote {
    pub kind: String,
    pub paths: Vec<String>,
    pub message: String,
}

pub const REBASE_PREDICTION_SCHEMA_VERSION: &str = "super-git.rebase-prediction.v0.1";

/// Stage 7 rebase-chain 충돌 예측 결과 (C9-C). merge 예측과 schema를 분리한
/// 이유: shape가 다르다(단일 prediction vs step 배열). schema_version이 shape를,
/// prediction_kind가 의미(merge/rebase)를 식별한다. 역시 plan이 아니다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebasePrediction {
    pub schema_version: String,
    /// 항상 "rebase".
    pub prediction_kind: String,
    pub repository: PathBuf,
    pub inputs: RebasePredictionInputs,
    /// oldest first. 첫 충돌 step까지만 들어 있다(이후 step은 예측하지 않음).
    pub steps: Vec<RebasePredictionStep>,
    pub summary: RebasePredictionSummary,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebasePredictionInputs {
    /// 재생 범위의 하한(이 커밋은 건드리지 않음). 범위는 base..head.
    pub base: ResolvedRev,
    /// 재생이 올라갈 새 시작점.
    pub onto: ResolvedRev,
    pub head: ResolvedRev,
    /// resolved oid 기준 "<base>..<head>" 표기. 표시용.
    pub range: String,
}

/// 커밋 하나의 replay 예측. C9-0 회전표 그대로:
/// merge base = 이 커밋의 parent, ours = 지금까지 합성된 tip, theirs = 이 커밋.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebasePredictionStep {
    pub commit: String,
    /// 3-way base로 쓴 이 커밋의 실제 parent.
    pub parent: String,
    pub prediction: ConflictPredictionOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebasePredictionSummary {
    /// "clean" | "conflicted".
    pub status: String,
    pub total_steps: u32,
    pub predicted_steps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_conflict_commit: Option<String>,
    /// 첫 충돌 이후 예측하지 않은 커밋 oid들(oldest first). 충돌 해결이
    /// 이후 모든 step을 바꾸므로 충돌 tree 위에 합성을 계속하지 않는다.
    pub steps_not_predicted: Vec<String>,
    /// 전 step clean일 때 rebase 후 예상되는 최종 트리 oid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_tree: Option<String>,
}
