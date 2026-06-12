//! Stage 7 충돌 예측 코어 (C9-A).
//! 계약: docs/internal/plans/2026-06-12-c9-0-conflict-prediction-contract.md
//!
//! `git merge-tree --write-tree`로 두 커밋의 병합을 object database 안에서만
//! 수행해 충돌을 예측한다. refs/index/워킹트리/설정은 절대 건드리지 않지만,
//! 참조되지 않는(gc 회수 가능) 오브젝트는 생성된다 — 문서에서 "read-only"를
//! 무조건으로 과장하지 않는 이유다. 자동 충돌 해결은 영구히 비목표.

use std::path::Path;

use crate::git::command::Git;
use crate::model::{
    ConflictPrediction, ConflictPredictionInputs, ConflictPredictionNote,
    ConflictPredictionOutcome, PredictedConflictFile, PredictedConflictStage, ResolvedRev,
    CONFLICT_PREDICTION_SCHEMA_VERSION,
};
use crate::{Result, SuperGitError};

const ACTION: &str = "conflict_prediction";

/// 한 commit pair의 merge 충돌을 예측한다. 예측된 충돌은 에러가 아니라
/// 성공한 예측이다(`prediction.status == "conflicted"`). 에러는 입력/환경
/// precondition(없는 rev, 공통 조상 없음, 너무 오래된 git)에만 쓴다.
pub fn predict_merge(
    current_path: &Path,
    ours_rev: &str,
    theirs_rev: &str,
) -> Result<ConflictPrediction> {
    let git = Git::default();
    let repository = git.run_path_in(current_path, ["rev-parse", "--show-toplevel"])?;

    let ours = resolve_commit(&git, &repository, "ours", ours_rev)?;
    let theirs = resolve_commit(&git, &repository, "theirs", theirs_rev)?;

    // 공통 조상이 없으면 merge-tree 자체가 거부한다. 더 명확한 코드로 먼저 알린다.
    let merge_base = read_merge_base(&git, &repository, &ours.commit, &theirs.commit)?;
    if merge_base.is_none() {
        return precondition(
            "no_merge_base",
            format!(
                "{} and {} share no common ancestor; merging unrelated histories is not supported",
                ours.rev, theirs.rev
            ),
        );
    }

    // resolve 단계에서 얻은 oid를 넘긴다: rev 표기 재해석 여지와 옵션 주입
    // 여지를 함께 없앤다(oid는 hex라 `-`로 시작할 수 없다).
    let args = [
        "merge-tree".to_string(),
        "--write-tree".to_string(),
        "-z".to_string(),
        ours.commit.clone(),
        theirs.commit.clone(),
    ];
    let result = git.try_run_in(&repository, args.iter().map(String::as_str))?;

    let conflicted = match result.status {
        Some(0) => false,
        Some(1) => true,
        // 129는 git의 usage error: --write-tree를 모르는 구버전(< 2.38)이 대표적.
        Some(129) => {
            return precondition(
                "merge_tree_unsupported",
                "this git does not support `merge-tree --write-tree`; git >= 2.38 is required",
            )
        }
        status => {
            return Err(SuperGitError::GitCommandFailed {
                args: args.to_vec(),
                status,
                stderr: result.stderr,
            })
        }
    };

    let parsed = parse_merge_tree_z(&result.stdout)?;
    // exit code와 출력 모양이 서로 다른 말을 하면 파서나 git이 틀린 것이다.
    if conflicted == parsed.files.is_empty() {
        return precondition(
            "merge_tree_output_unrecognized",
            "merge-tree exit status and conflicted-file output disagree; refusing to guess",
        );
    }

    Ok(ConflictPrediction {
        schema_version: CONFLICT_PREDICTION_SCHEMA_VERSION.to_string(),
        prediction_kind: "merge".to_string(),
        repository,
        inputs: ConflictPredictionInputs {
            ours,
            theirs,
            merge_base,
        },
        prediction: ConflictPredictionOutcome {
            status: if conflicted { "conflicted" } else { "clean" }.to_string(),
            merged_tree: parsed.tree,
            conflicted_files: parsed.files,
            notes: parsed.notes,
        },
        limitations: limitations(),
    })
}

/// 출력에 항상 실리는 과장 방지 문구. 소비자(에이전트)가 예측을 rebase
/// 트랜스크립트로 오해하는 것이 가장 위험한 오용이라 결과에 직접 넣는다.
fn limitations() -> Vec<String> {
    [
        "prediction is commit-level: the index and working tree are ignored",
        "a rebase replays commits one by one; its conflicts can differ from this single merge prediction",
        "note messages are localized free text; only `kind` and `paths` are stable",
    ]
    .map(String::from)
    .to_vec()
}

fn resolve_commit(git: &Git, repository: &Path, side: &str, rev: &str) -> Result<ResolvedRev> {
    // --end-of-options: rev가 `-`로 시작해도 옵션으로 해석되지 않게 한다.
    // ^{commit}으로 태그는 commit까지 peel하고, blob/tree rev는 거부한다.
    let result = git.try_run_in(
        repository,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ],
    )?;
    if !result.success {
        return precondition(
            "rev_not_found",
            format!("{side} revision {rev:?} does not resolve to a commit in this repository"),
        );
    }
    Ok(ResolvedRev {
        rev: rev.to_string(),
        commit: result.stdout.trim().to_string(),
    })
}

fn read_merge_base(
    git: &Git,
    repository: &Path,
    ours: &str,
    theirs: &str,
) -> Result<Option<String>> {
    let result = git.try_run_in(repository, ["merge-base", ours, theirs])?;
    if !result.success {
        return Ok(None);
    }
    Ok(Some(result.stdout.trim().to_string()))
}

struct ParsedMergeTree {
    tree: String,
    files: Vec<PredictedConflictFile>,
    notes: Vec<ConflictPredictionNote>,
}

/// `merge-tree --write-tree -z` 출력의 순수 파서. 실측으로 고정한 형태
/// (git 2.54, 로케일 무관 검증):
///
/// ```text
/// <tree-oid> NUL
/// <mode> SP <object> SP <stage> TAB <path> NUL   (0개 이상, path 순 정렬)
/// NUL                                            (빈 토큰 = 섹션 구분)
/// <N> NUL <path>{N} NUL <kind> NUL <message> NUL (0개 이상의 stanza)
/// ```
///
/// clean이면 tree oid 토큰 하나로 끝난다. 줄/로컬라이즈된 텍스트가 아니라
/// NUL 토큰만 읽으므로 로케일과 무관하다.
fn parse_merge_tree_z(stdout: &str) -> Result<ParsedMergeTree> {
    let mut tokens = stdout.split('\0');

    let tree = match tokens.next() {
        Some(oid) if !oid.is_empty() && oid.bytes().all(|b| b.is_ascii_hexdigit()) => {
            oid.to_string()
        }
        _ => return unrecognized("missing toplevel tree oid"),
    };

    // 충돌 파일 stage 정보 섹션: 빈 토큰(섹션 구분자) 또는 끝까지.
    let mut files: Vec<PredictedConflictFile> = Vec::new();
    for token in tokens.by_ref() {
        if token.is_empty() {
            break;
        }
        let (path, stage) = parse_file_info(token)?;
        // 출력이 path 순으로 정렬돼 같은 path의 stage들은 인접한다.
        match files.last_mut() {
            Some(file) if file.path == path => file.stages.push(stage),
            _ => files.push(PredictedConflictFile {
                path,
                stages: vec![stage],
            }),
        }
    }

    // informational stanza 섹션. message는 번역되는 자유 텍스트라 그대로 담되
    // 표시 전용이다(모델 주석 참고). kind/paths만 안정 계약이다.
    let mut notes = Vec::new();
    while let Some(token) = tokens.next() {
        if token.is_empty() {
            // 출력 끝의 NUL이 만든 마지막 빈 토큰.
            continue;
        }
        let count: usize = match token.parse() {
            Ok(count) => count,
            Err(_) => return unrecognized("informational stanza must start with a path count"),
        };
        let mut paths = Vec::with_capacity(count);
        for _ in 0..count {
            match tokens.next() {
                Some(path) if !path.is_empty() => paths.push(path.to_string()),
                _ => return unrecognized("informational stanza ended before its path list"),
            }
        }
        let (Some(kind), Some(message)) = (tokens.next(), tokens.next()) else {
            return unrecognized("informational stanza is missing kind or message");
        };
        notes.push(ConflictPredictionNote {
            kind: kind.to_string(),
            paths,
            message: message.to_string(),
        });
    }

    Ok(ParsedMergeTree { tree, files, notes })
}

/// `<mode> SP <object> SP <stage> TAB <path>` 한 토큰을 파싱한다.
fn parse_file_info(token: &str) -> Result<(String, PredictedConflictStage)> {
    let Some((meta, path)) = token.split_once('\t') else {
        return unrecognized("conflicted file info has no tab separator");
    };
    let mut fields = meta.split(' ');
    let (Some(mode), Some(object), Some(stage), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return unrecognized("conflicted file info must be `mode object stage`");
    };
    let stage: u8 = match stage.parse() {
        Ok(stage @ 1..=3) => stage,
        _ => return unrecognized("conflict stage must be 1, 2, or 3"),
    };
    Ok((
        path.to_string(),
        PredictedConflictStage {
            stage,
            mode: mode.to_string(),
            object: object.to_string(),
        },
    ))
}

fn precondition<T>(code: &str, message: impl Into<String>) -> Result<T> {
    Err(SuperGitError::PreviewPreconditionFailed {
        action: ACTION.to_string(),
        code: code.to_string(),
        message: message.into(),
    })
}

fn unrecognized<T>(detail: &str) -> Result<T> {
    precondition(
        "merge_tree_output_unrecognized",
        format!("could not parse `git merge-tree -z` output: {detail}"),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::{Command, Output};

    use super::{parse_merge_tree_z, predict_merge};
    use crate::SuperGitError;

    // ---- 순수 파서 테스트: git 출력 드리프트가 여기서 먼저 깨지게 한다 ----

    #[test]
    fn parses_clean_output_with_only_tree_oid() {
        let parsed = parse_merge_tree_z("9d176c96faaba89f50443126c2938cabb4d4e7f4\0").unwrap();
        assert_eq!(parsed.tree, "9d176c96faaba89f50443126c2938cabb4d4e7f4");
        assert!(parsed.files.is_empty());
        assert!(parsed.notes.is_empty());
    }

    #[test]
    fn parses_content_conflict_with_localized_message() {
        // git 2.54 + 한국어 로케일 실측 출력 fixture: message는 번역되지만
        // kind 토큰과 구조는 영어/NUL 그대로다.
        let raw = "eecdd9affe4f9ef640599bffc458c7af86c9f715\0\
                   100644 de980441c3ab03a8c07dda1ad27b8a11f39deb1e 1\tf.txt\0\
                   100644 b13fc29ced69ff0a120aea7222ba8adce9e4fe00 2\tf.txt\0\
                   100644 d166b8f8857e806e558d913de36f36b822bf81bf 3\tf.txt\0\
                   \0\
                   1\0f.txt\0Auto-merging\0자동 병합: f.txt\n\0\
                   1\0f.txt\0CONFLICT (contents)\0충돌 (내용): f.txt에 병합 충돌\n\0";
        let parsed = parse_merge_tree_z(raw).unwrap();

        assert_eq!(parsed.files.len(), 1);
        let file = &parsed.files[0];
        assert_eq!(file.path, "f.txt");
        let stages: Vec<u8> = file.stages.iter().map(|s| s.stage).collect();
        assert_eq!(stages, vec![1, 2, 3]);
        assert_eq!(file.stages[0].mode, "100644");
        assert_eq!(
            file.stages[2].object,
            "d166b8f8857e806e558d913de36f36b822bf81bf"
        );

        assert_eq!(parsed.notes.len(), 2);
        assert_eq!(parsed.notes[1].kind, "CONFLICT (contents)");
        assert_eq!(parsed.notes[1].paths, vec!["f.txt"]);
    }

    #[test]
    fn parses_modify_delete_with_missing_stage() {
        // modify/delete: 삭제된 쪽 stage(3)가 없다. 소비자는 이 부재로 분기한다.
        let raw = "103d15752168da5249b5db97a6947654bf99c66f\0\
                   100644 94053253aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1\tf.txt\0\
                   100644 834469cabbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 2\tf.txt\0\
                   \0\
                   1\0f.txt\0CONFLICT (modify/delete)\0CONFLICT message\n\0";
        let parsed = parse_merge_tree_z(raw).unwrap();
        let stages: Vec<u8> = parsed.files[0].stages.iter().map(|s| s.stage).collect();
        assert_eq!(stages, vec![1, 2]);
        assert_eq!(parsed.notes[0].kind, "CONFLICT (modify/delete)");
    }

    #[test]
    fn rejects_garbage_output() {
        assert!(parse_merge_tree_z("").is_err());
        assert!(parse_merge_tree_z("not-a-hex-oid\0").is_err());
        assert!(parse_merge_tree_z("abc123\0no-tab-here\0").is_err());
        assert!(parse_merge_tree_z("abc123\x00100644 oid 9\tf.txt\x00").is_err());
        // stanza가 count 뒤에서 끊기면 거부.
        assert!(parse_merge_tree_z("abc123\x00\x002\x00only-one-path\x00").is_err());
    }

    // ---- 실제 git 통합 테스트 ----

    fn run_git(dir: &Path, args: &[&str]) -> Output {
        Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .expect("run git")
    }

    fn git(dir: &Path, args: &[&str]) {
        let output = run_git(dir, args);
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).expect("write file");
    }

    /// main에서 갈라진 left/right 두 브랜치를 만든다. 충돌 여부는 내용이 정한다.
    fn repo_with_branches(dir: &Path, left_content: &str, right_content: &str) {
        git(dir, &["init", "-q", "-b", "main"]);
        write(dir, "f.txt", "a\nb\nc\nd\ne\n");
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", "init"]);
        git(dir, &["checkout", "-q", "-b", "left"]);
        write(dir, "f.txt", left_content);
        git(dir, &["commit", "-q", "-am", "left"]);
        git(dir, &["checkout", "-q", "main"]);
        git(dir, &["checkout", "-q", "-b", "right"]);
        write(dir, "f.txt", right_content);
        git(dir, &["commit", "-q", "-am", "right"]);
    }

    #[test]
    fn predicts_clean_merge_for_disjoint_edits() {
        let tmp = tempfile::tempdir().expect("temp dir");
        repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "a\nb\nc\nd\nRIGHT\n");

        let prediction = predict_merge(tmp.path(), "left", "right").expect("predict");

        assert_eq!(
            prediction.schema_version,
            "super-git.conflict-prediction.v0.1"
        );
        assert_eq!(prediction.prediction_kind, "merge");
        assert_eq!(prediction.prediction.status, "clean");
        assert!(prediction.prediction.conflicted_files.is_empty());
        assert!(!prediction.prediction.merged_tree.is_empty());
        assert_eq!(prediction.inputs.ours.rev, "left");
        assert_eq!(prediction.inputs.ours.commit.len(), 40);
        assert!(prediction.inputs.merge_base.is_some());
        assert!(!prediction.limitations.is_empty());
    }

    #[test]
    fn predicts_textual_conflict_with_full_stage_shape() {
        let tmp = tempfile::tempdir().expect("temp dir");
        repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "RIGHT\nb\nc\nd\ne\n");

        let prediction = predict_merge(tmp.path(), "left", "right").expect("predict");

        assert_eq!(prediction.prediction.status, "conflicted");
        assert_eq!(prediction.prediction.conflicted_files.len(), 1);
        let file = &prediction.prediction.conflicted_files[0];
        assert_eq!(file.path, "f.txt");
        let stages: Vec<u8> = file.stages.iter().map(|s| s.stage).collect();
        assert_eq!(stages, vec![1, 2, 3]);
        // kind 토큰은 로케일 무관 안정 계약이다.
        assert!(prediction
            .prediction
            .notes
            .iter()
            .any(|note| note.kind == "CONFLICT (contents)" && note.paths == ["f.txt"]));
    }

    #[test]
    fn predicts_modify_delete_conflict_by_stage_absence() {
        let tmp = tempfile::tempdir().expect("temp dir");
        repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "a\nb\nc\nd\nRIGHT\n");
        git(tmp.path(), &["checkout", "-q", "-b", "del", "main"]);
        git(tmp.path(), &["rm", "-q", "f.txt"]);
        git(tmp.path(), &["commit", "-q", "-m", "delete f.txt"]);

        let prediction = predict_merge(tmp.path(), "left", "del").expect("predict");

        assert_eq!(prediction.prediction.status, "conflicted");
        let file = &prediction.prediction.conflicted_files[0];
        let stages: Vec<u8> = file.stages.iter().map(|s| s.stage).collect();
        // 삭제된 쪽(theirs) stage 3가 없다 — 부재가 곧 충돌 모양이다.
        assert_eq!(stages, vec![1, 2]);
    }

    #[test]
    fn unknown_rev_is_a_structured_precondition_error() {
        let tmp = tempfile::tempdir().expect("temp dir");
        repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "a\nb\nc\nd\nRIGHT\n");

        let err = predict_merge(tmp.path(), "left", "no-such-branch").unwrap_err();
        match err {
            SuperGitError::PreviewPreconditionFailed { code, message, .. } => {
                assert_eq!(code, "rev_not_found");
                assert!(message.contains("theirs"));
            }
            other => panic!("expected precondition error, got {other:?}"),
        }
    }

    #[test]
    fn unrelated_histories_report_no_merge_base() {
        let tmp = tempfile::tempdir().expect("temp dir");
        repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "a\nb\nc\nd\nRIGHT\n");
        // 공통 조상이 없는 orphan 브랜치를 만든다.
        git(tmp.path(), &["checkout", "-q", "--orphan", "island"]);
        git(tmp.path(), &["rm", "-rfq", "."]);
        write(tmp.path(), "other.txt", "island\n");
        git(tmp.path(), &["add", "."]);
        git(tmp.path(), &["commit", "-q", "-m", "island root"]);

        let err = predict_merge(tmp.path(), "left", "island").unwrap_err();
        match err {
            SuperGitError::PreviewPreconditionFailed { code, .. } => {
                assert_eq!(code, "no_merge_base");
            }
            other => panic!("expected precondition error, got {other:?}"),
        }
    }

    #[test]
    fn rev_starting_with_dash_cannot_inject_options() {
        let tmp = tempfile::tempdir().expect("temp dir");
        repo_with_branches(tmp.path(), "LEFT\nb\nc\nd\ne\n", "a\nb\nc\nd\nRIGHT\n");

        // --end-of-options 덕분에 옵션이 아니라 rev로 해석되고, 없는 rev라 거부된다.
        let err = predict_merge(tmp.path(), "left", "--all").unwrap_err();
        match err {
            SuperGitError::PreviewPreconditionFailed { code, .. } => {
                assert_eq!(code, "rev_not_found");
            }
            other => panic!("expected precondition error, got {other:?}"),
        }
    }
}
