//! The single implementation of the stale-head rule (design §7): whether a
//! freshly normalized capture still names the same review basis a
//! previously observed `pr_observation.v0` recorded, and — when it does —
//! what changed at that unchanged head since the previous observation was
//! captured.
//!
//! `classify_refresh` never rebases: on any head, base, repository, or PR
//! mismatch it reports `stale_head` and emits **no** refreshed observation
//! hash (`review_basis_moved` is hardcoded `false`, mirroring the schema
//! `const`). Moving the declared review basis to a new head always requires
//! the operator to run `github observe` on the new capture, which mints a
//! visibly different `observation_id`/hash — never silent (acceptance
//! criterion 4).
//!
//! Trust boundary (design §6.1): the caller-supplied `previous` observation
//! is the one record contract this adapter accepts back as input — the
//! operator's *declared* review basis. Its `normalized_content_hash` is
//! recomputed (hash field cleared) and checked against the supplied value
//! **before any classification runs**; a mismatch is a hard refusal. Be
//! precise about what that proves: the record is self-consistent — its
//! claimed hash matches its own claimed bytes — not that this tool produced
//! it from real provider data. A fully self-consistent forgery remains
//! possible; the check exists so a tampered basis (e.g. a head SHA edited to
//! match the new capture, to dodge `stale_head`) cannot even reach
//! `classify_refresh`'s comparison, not to authenticate provenance. The
//! mitigation for provenance is that the operator chooses which basis to
//! declare, not this check.

use super::model::{
    CheckEvidence, ObservationChange, ObservationChangeKind, PrObservation, RefreshDisposition,
    RefreshResult, ReviewFinding, GITHUB_REFRESH_RESULT_SCHEMA,
};
use super::normalize::NormalizedCapture;
use crate::memory::MemoryValidationFinding;
use std::collections::{BTreeMap, BTreeSet};

fn refusal(code: &str, location: &str, detail: &str) -> MemoryValidationFinding {
    MemoryValidationFinding {
        code: code.to_owned(),
        location: location.to_owned(),
        detail: detail.to_owned(),
    }
}

/// Classifies a refresh against a previously observed `pr_observation` and a
/// freshly normalized new capture.
///
/// `previous_checks`/`previous_findings` are the `check_evidence.v0`/
/// `review_finding.v0` records `github observe` emitted alongside `previous`
/// at capture time. They are required to detect same-head drift
/// (disappearing/added/changed checks, an edited review comment, a thread
/// resolution flip; design §7) — a `pr_observation` alone carries no
/// per-check or per-finding state to diff against, only
/// repository/PR/base/head/liveness/changed_files. The design's CLI sketch
/// (§9) wires only `--previous-observation`; supplying these two additional
/// previous-capture inputs is a CLI (T5) concern the CLI must resolve (e.g.
/// an additional flag, or a bundled previous-capture directory) — passing
/// empty slices here still yields a correct, narrower refresh (disposition
/// and liveness drift only) if a first CLI cut needs that.
pub fn classify_refresh(
    previous: &PrObservation,
    previous_checks: &[CheckEvidence],
    previous_findings: &[ReviewFinding],
    current: &NormalizedCapture,
) -> Result<RefreshResult, MemoryValidationFinding> {
    // Basis integrity first (design §6.1/§7): a tampered previous
    // observation must never even reach the stale-head comparison below.
    // The single implementation of this computation lives in
    // `normalize.rs` (`observation_content_hash`, `pub(crate)`) — this
    // module used to keep a byte-for-byte copy, which CLAUDE.md's "a
    // decision rule has exactly one implementation" forbids even for a
    // mechanical computation like this one.
    let recomputed_hash = super::normalize::observation_content_hash(previous);
    if recomputed_hash != previous.normalized_content_hash {
        return Err(refusal(
            "previous_observation_hash_mismatch",
            "$.previous_observation.normalized_content_hash",
            "the supplied previous observation's declared hash does not match its own bytes",
        ));
    }

    let current_observation = &current.pr_observation;
    let same_review_basis = previous.repository == current_observation.repository
        && previous.pr.number == current_observation.pr.number
        && previous.base.sha == current_observation.base.sha
        && previous.head.sha == current_observation.head.sha;

    let previous_observation_hash = previous.normalized_content_hash.clone();
    let previous_head_sha = previous.head.sha.clone();
    let previous_base_sha = previous.base.sha.clone();
    let observed_head_sha = current_observation.head.sha.clone();
    let observed_base_sha = current_observation.base.sha.clone();

    if !same_review_basis {
        // A refresh cannot rebase by construction: no observation_changes,
        // no refreshed_observation_hash. Moving the basis requires the
        // operator to run `github observe` on the new capture instead.
        return Ok(RefreshResult {
            schema: GITHUB_REFRESH_RESULT_SCHEMA.to_owned(),
            previous_observation_hash,
            previous_head_sha,
            previous_base_sha,
            observed_head_sha,
            observed_base_sha,
            disposition: RefreshDisposition::StaleHead,
            review_basis_moved: false,
            observation_changes: Vec::new(),
            refreshed_observation_hash: None,
        });
    }

    let mut observation_changes = Vec::new();
    observation_changes.extend(liveness_changes(previous, current_observation));
    observation_changes.extend(check_changes(previous_checks, &current.check_evidence));
    observation_changes.extend(finding_changes(previous_findings, &current.review_findings));
    observation_changes.sort_by(|left, right| {
        (&left.category, &left.subject_id).cmp(&(&right.category, &right.subject_id))
    });

    Ok(RefreshResult {
        schema: GITHUB_REFRESH_RESULT_SCHEMA.to_owned(),
        previous_observation_hash,
        previous_head_sha,
        previous_base_sha,
        observed_head_sha,
        observed_base_sha,
        disposition: RefreshDisposition::HeadUnchanged,
        review_basis_moved: false,
        observation_changes,
        refreshed_observation_hash: Some(current_observation.normalized_content_hash.clone()),
    })
}

fn liveness_changes(previous: &PrObservation, current: &PrObservation) -> Vec<ObservationChange> {
    if previous.liveness == current.liveness {
        return Vec::new();
    }
    vec![ObservationChange {
        category: "liveness".to_owned(),
        change: ObservationChangeKind::Changed,
        subject_id: current.pr.number.to_string(),
        detail: format!(
            "liveness changed from {:?} to {:?}",
            previous.liveness, current.liveness
        ),
    }]
}

fn check_changes(previous: &[CheckEvidence], current: &[CheckEvidence]) -> Vec<ObservationChange> {
    let mut changes = Vec::new();
    let previous_by_id: BTreeMap<&str, &CheckEvidence> = previous
        .iter()
        .map(|check| (check.check_id.as_str(), check))
        .collect();
    let current_by_id: BTreeMap<&str, &CheckEvidence> = current
        .iter()
        .map(|check| (check.check_id.as_str(), check))
        .collect();

    for (check_id, check) in &previous_by_id {
        match current_by_id.get(check_id) {
            None => changes.push(ObservationChange {
                category: "checks".to_owned(),
                change: ObservationChangeKind::Removed,
                subject_id: (*check_id).to_owned(),
                detail: format!(
                    "check {} present in the previous observation is absent now",
                    check.name
                ),
            }),
            Some(current_check) if current_check != check => changes.push(ObservationChange {
                category: "checks".to_owned(),
                change: ObservationChangeKind::Changed,
                subject_id: (*check_id).to_owned(),
                detail: format!("check {} status/conclusion changed", check.name),
            }),
            Some(_) => {}
        }
    }
    for (check_id, check) in &current_by_id {
        if !previous_by_id.contains_key(check_id) {
            changes.push(ObservationChange {
                category: "checks".to_owned(),
                change: ObservationChangeKind::Added,
                subject_id: (*check_id).to_owned(),
                detail: format!("check {} newly observed at this head", check.name),
            });
        }
    }
    changes
}

/// Same-head drift on findings: an edited body on an existing `finding_id`
/// (design's "changed review comment" case), and a thread resolution flip
/// (deduped to one entry per thread, since several findings can share a
/// `thread_id`). Findings appearing only in `current` (new comments posted
/// since the previous capture) are not drift on an *existing* finding and
/// are intentionally not reported here — they simply appear in the new
/// capture's own full findings list.
fn finding_changes(
    previous: &[ReviewFinding],
    current: &[ReviewFinding],
) -> Vec<ObservationChange> {
    let mut changes = Vec::new();
    let previous_by_id: BTreeMap<&str, &ReviewFinding> = previous
        .iter()
        .map(|finding| (finding.finding_id.as_str(), finding))
        .collect();
    let mut flipped_threads: BTreeSet<String> = BTreeSet::new();

    for current_finding in current {
        let Some(previous_finding) = previous_by_id.get(current_finding.finding_id.as_str()) else {
            continue;
        };
        if previous_finding.body_content_hash != current_finding.body_content_hash {
            changes.push(ObservationChange {
                category: "review_findings".to_owned(),
                change: ObservationChangeKind::Changed,
                subject_id: current_finding.finding_id.clone(),
                detail: "body_content_hash changed since the previous observation".to_owned(),
            });
        }
        if let (Some(previous_thread), Some(current_thread)) =
            (&previous_finding.thread, &current_finding.thread)
        {
            if previous_thread.resolved != current_thread.resolved {
                flipped_threads.insert(current_thread.thread_id.clone());
            }
        }
    }
    for thread_id in flipped_threads {
        changes.push(ObservationChange {
            category: "review_threads".to_owned(),
            change: ObservationChangeKind::Changed,
            subject_id: thread_id,
            detail: "thread resolution state flipped since the previous observation".to_owned(),
        });
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_evidence::model::{
        CheckKind, FindingKind, ImplementationActors, Liveness, MergeableState, PrAuthor,
        PrObservationPr, RefSha, ReviewFindingAuthor, ReviewThreadState,
        GITHUB_CHECK_EVIDENCE_SCHEMA, GITHUB_PR_OBSERVATION_SCHEMA, GITHUB_REVIEW_FINDING_SCHEMA,
    };

    const REPO: &str = "OWNER/repo";
    const BASE_SHA: &str = "947f347f219a60775bcf71b226ce778cc8ea21f4";
    const HEAD_SHA: &str = "c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b";
    const OTHER_HEAD_SHA: &str = "5403673f13b45d8deb0f4be62f50390172071bb0";

    fn observation(head_sha: &str) -> PrObservation {
        let mut observation = PrObservation {
            schema: GITHUB_PR_OBSERVATION_SCHEMA.to_owned(),
            observation_id: format!("github-observation:{REPO}#101@{head_sha}"),
            repository: REPO.to_owned(),
            issues: Vec::new(),
            pr: PrObservationPr {
                number: 101,
                title: "Add a thing".to_owned(),
                url: "https://github.com/OWNER/repo/pull/101".to_owned(),
                state: "MERGED".to_owned(),
                author: PrAuthor {
                    id: "actor:pr-author".to_owned(),
                    login: "alice".to_owned(),
                },
                created_at: "2026-01-01T00:00:00Z".to_owned(),
                body_content_hash:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_owned(),
            },
            base: RefSha {
                git_ref: "main".to_owned(),
                sha: BASE_SHA.to_owned(),
            },
            head: RefSha {
                git_ref: "feature".to_owned(),
                sha: head_sha.to_owned(),
            },
            liveness: Liveness {
                state: "MERGED".to_owned(),
                mergeable: MergeableState::Unknown,
                merge_state_status: "UNKNOWN".to_owned(),
                merged_at: Some("2026-01-01T01:00:00Z".to_owned()),
                closed_at: None,
                merge_commit_sha: None,
            },
            changed_files: Vec::new(),
            implementation_actors: ImplementationActors {
                actor_ids: vec!["actor:pr-author".to_owned()],
                logins: vec!["alice".to_owned()],
                derivation: "pr_author_and_commit_authors_and_committers".to_owned(),
            },
            source_record_ids: Vec::new(),
            captured_at: "2026-01-01T00:00:00Z".to_owned(),
            provider_fields_unmapped: true,
            normalized_content_hash: String::new(),
        };
        observation.normalized_content_hash =
            super::super::normalize::observation_content_hash(&observation);
        observation
    }

    fn empty_totals() -> super::super::normalize::CaptureTotals {
        super::super::normalize::CaptureTotals {
            reviews_reported_total: 0,
            reviews_received: 0,
            review_threads_reported_total: 0,
            review_threads_received: 0,
            thread_comment_totals: Vec::new(),
        }
    }

    fn capture(
        observation: PrObservation,
        checks: Vec<CheckEvidence>,
        findings: Vec<ReviewFinding>,
    ) -> NormalizedCapture {
        NormalizedCapture {
            source_records: Vec::new(),
            pr_observation: observation,
            check_evidence: checks,
            review_findings: findings,
            domain_findings: Vec::new(),
            cross_repository_excluded: Vec::new(),
            capture_totals: empty_totals(),
        }
    }

    fn check(check_id: &str, conclusion: &str) -> CheckEvidence {
        CheckEvidence {
            schema: GITHUB_CHECK_EVIDENCE_SCHEMA.to_owned(),
            check_id: check_id.to_owned(),
            head_sha: HEAD_SHA.to_owned(),
            kind: CheckKind::CheckRun,
            name: "quality".to_owned(),
            workflow_name: None,
            status: Some("COMPLETED".to_owned()),
            conclusion: Some(conclusion.to_owned()),
            state: None,
            creator: None,
            details_url: None,
            target_url: None,
            description: None,
            started_at: None,
            completed_at: None,
            created_at: None,
            evidence_role: "ci_check".to_owned(),
            source_record_id: "github-source:checks:sha256-abc".to_owned(),
        }
    }

    fn thread_finding(
        finding_id: &str,
        body_hash: &str,
        thread_id: &str,
        resolved: bool,
    ) -> ReviewFinding {
        ReviewFinding {
            schema: GITHUB_REVIEW_FINDING_SCHEMA.to_owned(),
            finding_id: finding_id.to_owned(),
            kind: FindingKind::ThreadComment,
            author: ReviewFindingAuthor {
                id: Some("actor:bot-1".to_owned()),
                login: "reviewbot".to_owned(),
                typename: Some("Bot".to_owned()),
                association: "NONE".to_owned(),
            },
            authored_at: "2026-01-01T02:30:00Z".to_owned(),
            last_edited_at: None,
            edited: false,
            url: format!("https://github.com/OWNER/repo/pull/101#{finding_id}"),
            path: Some("a.rs".to_owned()),
            review_state: None,
            commit_sha: None,
            body_content_hash: body_hash.to_owned(),
            actionable: true,
            thread: Some(ReviewThreadState {
                thread_id: thread_id.to_owned(),
                resolved,
                outdated: false,
                resolved_by: None,
                comment_count: 1,
            }),
            duplicate_count: 1,
            source_record_id: "github-source:review_threads:sha256-abc".to_owned(),
        }
    }

    #[test]
    fn stale_head_on_different_head_sha_emits_no_refreshed_hash() {
        let previous = observation(HEAD_SHA);
        let current = capture(observation(OTHER_HEAD_SHA), Vec::new(), Vec::new());

        let result = classify_refresh(&previous, &[], &[], &current).expect("classify_refresh");
        assert_eq!(result.disposition, RefreshDisposition::StaleHead);
        assert!(!result.review_basis_moved);
        assert!(result.refreshed_observation_hash.is_none());
        assert!(result.observation_changes.is_empty());
        assert_eq!(result.observed_head_sha, OTHER_HEAD_SHA);
        assert_eq!(result.previous_head_sha, HEAD_SHA);
    }

    #[test]
    fn head_unchanged_with_nothing_else_changed() {
        let previous = observation(HEAD_SHA);
        let current = capture(observation(HEAD_SHA), Vec::new(), Vec::new());

        let result = classify_refresh(&previous, &[], &[], &current).expect("classify_refresh");
        assert_eq!(result.disposition, RefreshDisposition::HeadUnchanged);
        assert!(!result.review_basis_moved);
        assert!(result.observation_changes.is_empty());
        assert_eq!(
            result.refreshed_observation_hash,
            Some(current.pr_observation.normalized_content_hash.clone())
        );
    }

    #[test]
    fn disappearing_check_is_reported_as_removed() {
        let previous = observation(HEAD_SHA);
        let previous_checks = vec![check("check:a", "SUCCESS"), check("check:b", "SUCCESS")];
        let current = capture(
            observation(HEAD_SHA),
            vec![check("check:a", "SUCCESS")],
            Vec::new(),
        );

        let result =
            classify_refresh(&previous, &previous_checks, &[], &current).expect("classify_refresh");
        assert_eq!(result.disposition, RefreshDisposition::HeadUnchanged);
        assert_eq!(
            result.observation_changes,
            vec![ObservationChange {
                category: "checks".to_owned(),
                change: ObservationChangeKind::Removed,
                subject_id: "check:b".to_owned(),
                detail: "check quality present in the previous observation is absent now"
                    .to_owned(),
            }]
        );
    }

    #[test]
    fn edited_review_comment_is_reported_as_changed() {
        let previous = observation(HEAD_SHA);
        let previous_findings = vec![thread_finding(
            "finding:1",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "thread-1",
            false,
        )];
        let current_findings = vec![thread_finding(
            "finding:1",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "thread-1",
            false,
        )];
        let current = capture(observation(HEAD_SHA), Vec::new(), current_findings);

        let result = classify_refresh(&previous, &[], &previous_findings, &current)
            .expect("classify_refresh");
        assert_eq!(
            result.observation_changes,
            vec![ObservationChange {
                category: "review_findings".to_owned(),
                change: ObservationChangeKind::Changed,
                subject_id: "finding:1".to_owned(),
                detail: "body_content_hash changed since the previous observation".to_owned(),
            }]
        );
    }

    #[test]
    fn thread_resolution_flip_is_reported_once_per_thread() {
        let previous = observation(HEAD_SHA);
        let previous_findings = vec![thread_finding(
            "finding:1",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "thread-1",
            false,
        )];
        let current_findings = vec![thread_finding(
            "finding:1",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "thread-1",
            true,
        )];
        let current = capture(observation(HEAD_SHA), Vec::new(), current_findings);

        let result = classify_refresh(&previous, &[], &previous_findings, &current)
            .expect("classify_refresh");
        assert_eq!(
            result.observation_changes,
            vec![ObservationChange {
                category: "review_threads".to_owned(),
                change: ObservationChangeKind::Changed,
                subject_id: "thread-1".to_owned(),
                detail: "thread resolution state flipped since the previous observation".to_owned(),
            }]
        );
    }

    #[test]
    fn tampered_previous_observation_is_refused_before_classification_runs() {
        let mut tampered = observation(HEAD_SHA);
        // Dodge attempt: edit the head SHA to match the new capture without
        // recomputing the hash, hoping to avoid a stale_head report.
        tampered.head.sha = OTHER_HEAD_SHA.to_owned();
        let current = capture(observation(OTHER_HEAD_SHA), Vec::new(), Vec::new());

        let error = classify_refresh(&tampered, &[], &[], &current)
            .expect_err("a tampered previous observation must refuse");
        assert_eq!(error.code, "previous_observation_hash_mismatch");
    }
}
