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
pub const HISTORY_EDIT_PLAN_SCHEMA_VERSION: &str = "super-git.plan.v0.4";
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
    /// 참고용 git 명령(canonical reference). 실행 허가가 아니라 문서화용 예시다.
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

/// `super-git.plan.v0.4` 히스토리 편집 계획.
/// 첫 op 세트(pick/reword/squash/fixup)는 트리를 보존하므로 분기 ref만 이동한다.
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
    pub result_summary: Option<HistoryEditResultSummaryView>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEditResultSummaryView {
    pub commits_before: u32,
    pub commits_after: u32,
    pub messages_changed: u32,
    pub commits_folded: u32,
    pub final_tree_unchanged: bool,
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
