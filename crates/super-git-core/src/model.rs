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

/// 저장소의 현재 상태 스냅샷. `inspect`의 핵심 모델.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepoState {
    /// 저장소(워크트리) 루트의 절대경로. 입력이 하위 디렉토리여도 root로 정규화된다.
    pub root: PathBuf,
    pub head: HeadInfo,
    pub operation: Operation,
}
