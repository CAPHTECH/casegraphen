//! `pr_observation` + `check_evidence` + `review_finding` + `review_independence`
//! → the compact `review_projection.v0` (design §8).
//!
//! The governing constraint (issue #102): a compact projection must not hide
//! a source-trace gap, a gluing failure, or an unsupported verification
//! claim. Every subject that lands in `must_review`/`should_review` keeps
//! its own reason and its own subject ids — grouping never merges two
//! distinguishable reasons into one vague label, and `losses`/
//! `residual_risks` are computed independently of the tier rule so a
//! declared loss can never be tiered away.
//!
//! This module reads decisions T3 already made rather than recomputing them:
//! "is this actionable finding unresolved" comes from
//! `ReviewIndependence::{unresolved,resolved}_actionable_finding_ids`, and
//! "who is this subject's verification source" comes from
//! `ReviewIndependence::classifications` — CLAUDE.md's "a decision rule has
//! exactly one implementation" forbids a second, parallel judgement of
//! either question here. Likewise, truncation is read from `normalize.rs`'s
//! `CaptureTotals` rather than re-parsing raw provider captures (which would
//! put provider parsing in two places).

use super::model::*;
use super::normalize::CaptureTotals;
use crate::native_hash::sha256_hex;
use std::collections::{BTreeMap, BTreeSet};

/// One subject (a finding id, a check id, or a synthetic policy/refresh id)
/// tagged with the tier the rule below assigned it and the one-line reason
/// for that assignment. Not a wire type — `ReviewProjection` carries
/// `must_review`/`should_review`/`can_skim` as three separate arrays
/// instead; this is `projection.rs`'s internal bookkeeping before it groups
/// assignments into `TierItem`s (mirrors the `ReviewTier` helper vocabulary
/// `model.rs` documents for exactly this purpose).
#[derive(Clone, Debug)]
struct Assignment {
    subject_id: String,
    tier: ReviewTier,
    path: Option<String>,
    reason: String,
}

/// Projects one PR observation, its checks, its findings, and its
/// independence classification into the compact Must/Should/Can reviewer
/// view (design §8). `capture_totals` is T2's already-computed provider
/// truncation evidence (never re-derived from raw artifacts here).
/// `stale_head_refresh` is optional: when a caller holds a `stale_head`
/// `RefreshResult` for this observation, everything that refresh names is
/// projected as Must Review rather than silently trusted (design §8,
/// "anything referenced by a `stale_head` refresh handed to projection").
/// `cross_repository_excluded` is `normalize.rs`'s structured record of every
/// finding URL dropped for naming a different repository than the manifest
/// declares (design §7) — read back here, never re-derived, so the
/// `cross_repository_excluded` loss below cites exactly what normalization
/// actually excluded.
pub fn project_review(
    observation: &PrObservation,
    checks: &[CheckEvidence],
    findings: &[ReviewFinding],
    independence: &ReviewIndependence,
    capture_totals: &CaptureTotals,
    cross_repository_excluded: &[String],
    stale_head_refresh: Option<&RefreshResult>,
) -> ReviewProjection {
    let mut assignments = assign_finding_tiers(findings, independence);
    let (check_assignments, mut failed_checks, mut inconclusive_checks) =
        assign_check_tiers(checks);
    assignments.extend(check_assignments);
    failed_checks.sort();
    inconclusive_checks.sort();

    if let Some(policy_assignment) = policy_assignment(observation, independence) {
        assignments.push(policy_assignment);
    }
    if let Some(refresh) = stale_head_refresh {
        if refresh.disposition == RefreshDisposition::StaleHead {
            assignments.push(stale_head_assignment(refresh));
        }
    }

    let must_review = group_into_items(&assignments, ReviewTier::MustReview);
    let should_review = group_into_items(&assignments, ReviewTier::ShouldReview);
    let can_skim = can_skim_items(observation, findings, capture_totals);

    let blocking_findings = to_finding_reasons(&assignments, ReviewTier::MustReview);
    let non_blocking_findings = to_finding_reasons(&assignments, ReviewTier::ShouldReview);

    let mut residual_risks = Vec::new();
    // Always surfaced when there is no independent human approval, whether
    // or not `require_independent_review` is set — the blocking finding
    // above is the separate, policy-gated consequence of the same fact.
    if independence.independent_human_approvals.is_empty() {
        residual_risks.push(ResidualRisk {
            code: "no_independent_human_approval".to_owned(),
            detail: "no independent human approval is bound to the observed head".to_owned(),
        });
    }
    if !inconclusive_checks.is_empty() {
        residual_risks.push(ResidualRisk {
            code: "checks_inconclusive".to_owned(),
            detail: format!(
                "{} check(s) produced no verification evidence: {}",
                inconclusive_checks.len(),
                inconclusive_checks.join(", ")
            ),
        });
    }
    if capture_totals.contexts_received < capture_totals.contexts_reported_total {
        residual_risks.push(ResidualRisk {
            code: "checks_capture_truncated".to_owned(),
            detail: format!(
                "checks contexts connection reported {} but the capture received {}; the \
                 check/status set this projection tiers is incomplete",
                capture_totals.contexts_reported_total, capture_totals.contexts_received
            ),
        });
    }
    residual_risks.extend(status_context_residual_risks(checks));

    let losses = losses(
        observation,
        findings,
        capture_totals,
        cross_repository_excluded,
    );

    let mut check_ids: Vec<String> = checks.iter().map(|check| check.check_id.clone()).collect();
    check_ids.sort();
    let mut finding_ids: Vec<String> = findings.iter().map(|f| f.finding_id.clone()).collect();
    finding_ids.sort();

    let mut verification_sources: Vec<VerificationSource> = independence
        .classifications
        .iter()
        .map(|classification| VerificationSource {
            subject_id: classification.subject_id.clone(),
            evidence_role: classification.evidence_role,
        })
        .collect();
    verification_sources.sort_by(|left, right| left.subject_id.cmp(&right.subject_id));

    // S10: derived from `capture_totals.thread_comment_totals`, not only
    // from `findings` — a thread whose comments were truncated to zero
    // received no comment finding at all, so deriving this solely from
    // `finding.thread` (as before) silently dropped it here even though
    // `normalize.rs`'s per-thread bookkeeping already knows its resolution
    // state independent of whether any comment survived.
    let mut unresolved_threads: Vec<String> = findings
        .iter()
        .filter_map(|f| f.thread.as_ref())
        .filter(|thread| !thread.resolved)
        .map(|thread| thread.thread_id.clone())
        .chain(
            capture_totals
                .thread_comment_totals
                .iter()
                .filter(|thread_totals| !thread_totals.resolved)
                .map(|thread_totals| thread_totals.thread_id.clone()),
        )
        .collect();
    unresolved_threads.sort();
    unresolved_threads.dedup();

    let mut source_record_ids = observation.source_record_ids.clone();
    source_record_ids.sort();

    let projection_id = format!(
        "github-projection:{}#{}@{}",
        observation.repository, observation.pr.number, observation.head.sha
    );

    let mut projection = ReviewProjection {
        schema: GITHUB_REVIEW_PROJECTION_SCHEMA.to_owned(),
        projection_id,
        pr_observation_hash: observation.normalized_content_hash.clone(),
        repository: observation.repository.clone(),
        pr_number: observation.pr.number,
        base_sha: observation.base.sha.clone(),
        head_sha: observation.head.sha.clone(),
        liveness: observation.liveness.clone(),
        must_review,
        should_review,
        can_skim,
        blocking_findings,
        non_blocking_findings,
        unresolved_threads,
        failed_checks,
        inconclusive_checks,
        verification_sources,
        residual_risks,
        losses,
        full_trace: FullTrace {
            source_record_ids,
            pr_observation_hash: observation.normalized_content_hash.clone(),
            check_ids,
            finding_ids,
            independence_included: true,
        },
        projection_content_hash: String::new(),
        read_only: true,
        accepted: false,
    };
    projection.projection_content_hash = projection_content_hash(&projection);
    projection
}

// ---------------------------------------------------------------------
// Finding tiering — priority order: unresolved actionable > resolved
// actionable > edited > independent-human-candidate (not otherwise tiered)
// > self/bot/unattributed-only verification. A subject is assigned at most
// once, by the first arm that matches, so `reason` never has to describe
// more than one fact about the same subject.
//
// Every arm here must be exhaustive over `EvidenceRole` between them and
// the four arms above (S9): `CiCheck` never reaches this function (it has
// no `ReviewFinding`, so `findings_by_id.contains_key` always excludes it —
// `assign_check_tiers` is its own, separate rule), but `SelfReview`,
// `AutomatedBot`, `Unattributed`, and `IndependentHumanCandidate` all name
// findings and must each land in exactly one tier or be caught by an
// actionable/edited arm above. Before this arm existed,
// `IndependentHumanCandidate` had no arm of its own: a non-actionable
// review summary from an independent human (every `review_summary` finding
// is `actionable: false` by construction, `normalize.rs`) fell through
// every arm and reached no tier, no `blocking_findings`, and no
// `non_blocking_findings` — the only trace left was a `verification_sources`
// entry that reads as a *positive* signal, which is the exact inverse of
// what an unresolved objection from an independent human means.
// ---------------------------------------------------------------------

fn assign_finding_tiers(
    findings: &[ReviewFinding],
    independence: &ReviewIndependence,
) -> Vec<Assignment> {
    let findings_by_id: BTreeMap<&str, &ReviewFinding> = findings
        .iter()
        .map(|finding| (finding.finding_id.as_str(), finding))
        .collect();

    let mut assigned: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();

    let mut unresolved: Vec<String> = independence.unresolved_actionable_finding_ids.clone();
    unresolved.sort();
    for finding_id in unresolved {
        let path = findings_by_id
            .get(finding_id.as_str())
            .and_then(|finding| finding.path.clone());
        out.push(Assignment {
            subject_id: finding_id.clone(),
            tier: ReviewTier::MustReview,
            path,
            reason: "unresolved actionable review finding".to_owned(),
        });
        assigned.insert(finding_id);
    }

    let mut resolved: Vec<String> = independence.resolved_actionable_finding_ids.clone();
    resolved.sort();
    for finding_id in resolved {
        if !assigned.insert(finding_id.clone()) {
            continue;
        }
        let path = findings_by_id
            .get(finding_id.as_str())
            .and_then(|finding| finding.path.clone());
        out.push(Assignment {
            subject_id: finding_id,
            tier: ReviewTier::ShouldReview,
            path,
            reason: "resolved actionable finding — verify the recorded resolution".to_owned(),
        });
    }

    let mut edited_ids: Vec<&str> = findings
        .iter()
        .filter(|finding| finding.edited)
        .map(|finding| finding.finding_id.as_str())
        .collect();
    edited_ids.sort_unstable();
    for finding_id in edited_ids {
        if assigned.contains(finding_id) {
            continue;
        }
        assigned.insert(finding_id.to_owned());
        let path = findings_by_id
            .get(finding_id)
            .and_then(|finding| finding.path.clone());
        out.push(Assignment {
            subject_id: finding_id.to_owned(),
            tier: ReviewTier::ShouldReview,
            path,
            reason: "finding body was edited after it was authored".to_owned(),
        });
    }

    // S9: an independent human's evidence that no arm above already claimed
    // (i.e. not itself an unresolved/resolved actionable thread comment or
    // an edited finding — a plain review summary is never actionable, so
    // this is the common case for a review's own top-level `APPROVED`/
    // `COMMENTED`/`CHANGES_REQUESTED` verdict). Split on whether it is the
    // specific approval `evaluate_independence` counted as satisfying the
    // policy: a satisfying approval is good news a human should still
    // verify, so it is Should Review; anything else — a `COMMENTED` or
    // `CHANGES_REQUESTED` verdict, or an `APPROVED` review excluded for not
    // binding to the observed head — is an independent human's input that
    // nothing else in this projection resolved, so it is Must Review.
    let mut independent_human_unresolved: Vec<&Classification> = independence
        .classifications
        .iter()
        .filter(|classification| {
            classification.evidence_role == EvidenceRole::IndependentHumanCandidate
                && findings_by_id.contains_key(classification.subject_id.as_str())
        })
        .collect();
    independent_human_unresolved.sort_by(|left, right| left.subject_id.cmp(&right.subject_id));
    for classification in independent_human_unresolved {
        if !assigned.insert(classification.subject_id.clone()) {
            continue;
        }
        let path = findings_by_id
            .get(classification.subject_id.as_str())
            .and_then(|finding| finding.path.clone());
        let is_satisfying_approval = independence
            .independent_human_approvals
            .iter()
            .any(|finding_id| finding_id == &classification.subject_id);
        let (tier, reason) = if is_satisfying_approval {
            (
                ReviewTier::ShouldReview,
                "independent human approval satisfying the independent-review policy — \
                 verify it is well-founded"
                    .to_owned(),
            )
        } else {
            (
                ReviewTier::MustReview,
                "independent human evidence not resolved by any other tier — review it \
                 directly"
                    .to_owned(),
            )
        };
        out.push(Assignment {
            subject_id: classification.subject_id.clone(),
            tier,
            path,
            reason,
        });
    }

    let mut self_bot_unattributed: Vec<&Classification> = independence
        .classifications
        .iter()
        .filter(|classification| {
            matches!(
                classification.evidence_role,
                EvidenceRole::SelfReview | EvidenceRole::AutomatedBot | EvidenceRole::Unattributed
            ) && findings_by_id.contains_key(classification.subject_id.as_str())
        })
        .collect();
    self_bot_unattributed.sort_by(|left, right| left.subject_id.cmp(&right.subject_id));
    for classification in self_bot_unattributed {
        if !assigned.insert(classification.subject_id.clone()) {
            continue;
        }
        let path = findings_by_id
            .get(classification.subject_id.as_str())
            .and_then(|finding| finding.path.clone());
        out.push(Assignment {
            subject_id: classification.subject_id.clone(),
            tier: ReviewTier::ShouldReview,
            path,
            reason: format!(
                "only verification source is {}",
                evidence_role_label(classification.evidence_role)
            ),
        });
    }

    out
}

fn evidence_role_label(role: EvidenceRole) -> &'static str {
    match role {
        EvidenceRole::SelfReview => "self-review",
        EvidenceRole::AutomatedBot => "an automated bot",
        EvidenceRole::Unattributed => "an unattributed actor",
        EvidenceRole::CiCheck => "a CI check",
        EvidenceRole::IndependentHumanCandidate => "an independent human candidate",
    }
}

// ---------------------------------------------------------------------
// Check tiering — three-way (design §3.7): `Success` gets no tier and no
// entry in either id list; `Failed` is Must Review and blocking;
// `Inconclusive` is Should Review and non-blocking, but is never treated as
// success — folding it into success would let a projection read clean when
// a verification simply never ran.
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckDisposition {
    Success,
    Failed,
    Inconclusive,
}

const FAILED_VALUES: &[&str] = &["FAILURE", "ERROR", "TIMED_OUT", "ACTION_REQUIRED"];
// `PENDING` (StatusContext) and `EXPECTED` (StatusContext) are the
// commit-status analogues of a CheckRun with no `conclusion` yet: the
// verification has not concluded, so it goes here, not into `Failed` and
// not silently into `Success`.
const INCONCLUSIVE_VALUES: &[&str] = &["NEUTRAL", "SKIPPED", "CANCELLED", "PENDING", "EXPECTED"];
const SUCCESS_VALUES: &[&str] = &["SUCCESS"];

/// The verbatim provider string this check's disposition is read from:
/// `conclusion` for a `CheckRun`, `state` for a `StatusContext` (design
/// §3.3/§3.7). `None` means the check has not concluded (still
/// `QUEUED`/`IN_PROGRESS`), which is `Inconclusive`, not success.
fn verbatim_conclusion(check: &CheckEvidence) -> Option<&str> {
    match check.kind {
        CheckKind::CheckRun => check.conclusion.as_deref(),
        CheckKind::StatusContext => check.state.as_deref(),
    }
}

/// An unrecognized verbatim value is `Inconclusive`, not `Success` and not
/// `Failed`: silently treating an unfamiliar provider string as success
/// would hide a gap, and blocking on it would be an unsupported claim in
/// the other direction. Either way the verbatim string itself is preserved
/// in the check record and in this subject's tier reason — nothing here
/// replaces it.
fn check_disposition(check: &CheckEvidence) -> CheckDisposition {
    match verbatim_conclusion(check) {
        None => CheckDisposition::Inconclusive,
        Some(value) if SUCCESS_VALUES.contains(&value) => CheckDisposition::Success,
        Some(value) if FAILED_VALUES.contains(&value) => CheckDisposition::Failed,
        Some(value) if INCONCLUSIVE_VALUES.contains(&value) => CheckDisposition::Inconclusive,
        Some(_) => CheckDisposition::Inconclusive,
    }
}

fn assign_check_tiers(checks: &[CheckEvidence]) -> (Vec<Assignment>, Vec<String>, Vec<String>) {
    let mut assignments = Vec::new();
    let mut failed = Vec::new();
    let mut inconclusive = Vec::new();
    for check in checks {
        match check_disposition(check) {
            CheckDisposition::Failed => {
                failed.push(check.check_id.clone());
                assignments.push(Assignment {
                    subject_id: check.check_id.clone(),
                    tier: ReviewTier::MustReview,
                    path: None,
                    reason: format!(
                        "check {} reported {}",
                        check.name,
                        verbatim_conclusion(check).unwrap_or("a non-success result")
                    ),
                });
            }
            CheckDisposition::Inconclusive => {
                inconclusive.push(check.check_id.clone());
                assignments.push(Assignment {
                    subject_id: check.check_id.clone(),
                    tier: ReviewTier::ShouldReview,
                    path: None,
                    reason: format!(
                        "check {} produced no verification evidence ({})",
                        check.name,
                        verbatim_conclusion(check).unwrap_or("no conclusion reported")
                    ),
                });
            }
            CheckDisposition::Success => {}
        }
    }
    (assignments, failed, inconclusive)
}

/// `StatusContext.description` is surfaced as a residual risk regardless of
/// that check's own disposition — the pilot's CodeRabbit status is
/// `state: SUCCESS` with `description: "Review rate limited"`, and a
/// projection that reads clean only because it dropped that description
/// would be exactly the hidden verification gap the issue forbids.
fn status_context_residual_risks(checks: &[CheckEvidence]) -> Vec<ResidualRisk> {
    let mut risks = Vec::new();
    for check in checks
        .iter()
        .filter(|check| check.kind == CheckKind::StatusContext)
    {
        if let Some(description) = &check.description {
            risks.push(ResidualRisk {
                code: "status_context_description".to_owned(),
                detail: format!("{}: {description}", check.name),
            });
        }
    }
    risks.sort_by(|left, right| left.detail.cmp(&right.detail));
    risks
}

// ---------------------------------------------------------------------
// Independent-review policy and stale-head refresh — each contributes at
// most one synthetic Must Review subject, since neither is a finding or a
// check.
// ---------------------------------------------------------------------

fn policy_assignment(
    observation: &PrObservation,
    independence: &ReviewIndependence,
) -> Option<Assignment> {
    if independence.policy.require_independent_review && !independence.policy.satisfied {
        Some(Assignment {
            subject_id: format!("independent_review_policy:{}", observation.observation_id),
            tier: ReviewTier::MustReview,
            path: None,
            reason: "require_independent_review is set and no independent human approval is \
                     bound to the observed head"
                .to_owned(),
        })
    } else {
        None
    }
}

/// The literal reading of design §8's "anything referenced by a
/// `stale_head` refresh handed to projection": a `stale_head`
/// `RefreshResult` carries no `observation_changes` (§3.6 — those are
/// populated only for same-head drift), so the only things it actually
/// references are the previous and observed head shas themselves. Both are
/// surfaced as one blocking subject rather than invented as a blanket
/// "re-review everything" rule this record does not support.
fn stale_head_assignment(refresh: &RefreshResult) -> Assignment {
    Assignment {
        subject_id: format!(
            "stale_head_refresh:{}->{}",
            refresh.previous_head_sha, refresh.observed_head_sha
        ),
        tier: ReviewTier::MustReview,
        path: None,
        reason: format!(
            "review basis is stale: a refresh observed head {} but this review basis is head {}",
            refresh.observed_head_sha, refresh.previous_head_sha
        ),
    }
}

// ---------------------------------------------------------------------
// Can Skim — changed files with no finding at that path, in either tier.
// ---------------------------------------------------------------------

/// `can_skim`'s claim is affirmative ("no review findings recorded against
/// this file"), so it must be entailed by the finding set actually present,
/// never by its mere absence (S10). A path is excluded from consideration
/// here — never labelled skimmable — whenever `capture_totals` shows a
/// thread at that path whose comments were not fully received: the
/// truncation means findings *were* withheld, not that none exist, and
/// `can_skim`'s wording would otherwise misstate a known gap as a clean
/// file. `path` is only ever `None` on a thread when the provider's own
/// `path` field is null (a PR-level, not file-level, thread); such threads
/// carry no file to exclude and are covered instead by the
/// `threads_truncated` loss and `unresolved_threads` alone.
fn can_skim_items(
    observation: &PrObservation,
    findings: &[ReviewFinding],
    capture_totals: &CaptureTotals,
) -> Vec<TierItem> {
    let touched: BTreeSet<&str> = findings
        .iter()
        .filter_map(|finding| finding.path.as_deref())
        .collect();
    let truncated_paths: BTreeSet<&str> = capture_totals
        .thread_comment_totals
        .iter()
        .filter(|thread_totals| thread_totals.received < thread_totals.reported_total)
        .filter_map(|thread_totals| thread_totals.path.as_deref())
        .collect();
    let mut items: Vec<TierItem> = observation
        .changed_files
        .iter()
        .filter(|file| {
            !touched.contains(file.path.as_str()) && !truncated_paths.contains(file.path.as_str())
        })
        .map(|file| TierItem {
            path: Some(file.path.clone()),
            subject_ids: Vec::new(),
            reason: "no review findings recorded against this file".to_owned(),
        })
        .collect();
    items.sort_by(|left, right| left.path.cmp(&right.path));
    items
}

// ---------------------------------------------------------------------
// Grouping assignments into the wire `TierItem`/`FindingReason` shapes.
// ---------------------------------------------------------------------

fn group_into_items(assignments: &[Assignment], tier: ReviewTier) -> Vec<TierItem> {
    let mut groups: BTreeMap<(Option<String>, String), Vec<String>> = BTreeMap::new();
    for assignment in assignments
        .iter()
        .filter(|assignment| assignment.tier == tier)
    {
        groups
            .entry((assignment.path.clone(), assignment.reason.clone()))
            .or_default()
            .push(assignment.subject_id.clone());
    }
    groups
        .into_iter()
        .map(|((path, reason), mut subject_ids)| {
            subject_ids.sort();
            TierItem {
                path,
                subject_ids,
                reason,
            }
        })
        .collect()
}

fn to_finding_reasons(assignments: &[Assignment], tier: ReviewTier) -> Vec<FindingReason> {
    let mut items: Vec<FindingReason> = assignments
        .iter()
        .filter(|assignment| assignment.tier == tier)
        .map(|assignment| FindingReason {
            finding_id: assignment.subject_id.clone(),
            reason: assignment.reason.clone(),
        })
        .collect();
    items.sort_by(|left, right| left.finding_id.cmp(&right.finding_id));
    items
}

// ---------------------------------------------------------------------
// Declared loss (design §8) — never empty. `bodies_hashed`,
// `provider_fields_unmapped`, and `files_completeness_unverifiable` are
// standing losses of this adapter family, emitted unconditionally;
// `reviews_truncated`/`threads_truncated` are derived from T2's
// `CaptureTotals`, never re-parsed from raw artifacts.
// ---------------------------------------------------------------------

fn losses(
    observation: &PrObservation,
    findings: &[ReviewFinding],
    capture_totals: &CaptureTotals,
    cross_repository_excluded: &[String],
) -> Vec<ProjectionLoss> {
    let mut body_refs: Vec<String> = vec![observation.pr.url.clone()];
    body_refs.extend(observation.issues.iter().map(|issue| issue.url.clone()));
    body_refs.extend(findings.iter().map(|finding| finding.finding_id.clone()));
    body_refs.sort();

    let mut losses = vec![
        ProjectionLoss {
            loss_kind: "bodies_hashed".to_owned(),
            detail: "issue/PR/review/comment bodies are represented only as content hashes; \
                     full text is retained in the source records cited by full_trace"
                .to_owned(),
            omitted_refs: body_refs,
        },
        ProjectionLoss {
            loss_kind: "provider_fields_unmapped".to_owned(),
            detail: "the capture allowlists only the fields this adapter reads; every other \
                     provider field is unmapped and not represented in this projection"
                .to_owned(),
            omitted_refs: vec![observation.observation_id.clone()],
        },
        // A declared blind spot, not a detected truncation, and it does not
        // depend on detection: `gh pr view --json files` returns a bare
        // array with no total count, so this adapter has no way to learn
        // whether the provider silently omitted files from that array.
        // `can_skim` reads as a complete changed-file list; without this
        // loss a reviewer who trusts that completeness has no way to learn
        // otherwise — exactly the hidden source-trace gap the issue
        // forbids. Emitted unconditionally: a short `changed_files` is not
        // evidence the list is actually complete.
        ProjectionLoss {
            loss_kind: "files_completeness_unverifiable".to_owned(),
            detail: "the files capture (gh pr view --json files) carries no total count, so \
                     this adapter cannot verify the changed-file list is complete; a short \
                     list is not evidence of a small change"
                .to_owned(),
            omitted_refs: vec![observation.observation_id.clone()],
        },
    ];
    // Cross-repository exclusions are a source-trace gap, not a truncation:
    // normalize.rs already dropped these findings entirely (design §7), so
    // the projection record itself — read alone, without the command
    // envelope's `domain_findings` around it — must still say so; a
    // reviewer holding only `review_projection.v0.json` has no other way to
    // learn a finding was excluded. Never re-derived here: `normalize.rs` is
    // the single implementation of *which* findings were cross-repository.
    if !cross_repository_excluded.is_empty() {
        losses.push(ProjectionLoss {
            loss_kind: "cross_repository_excluded".to_owned(),
            detail: "review findings whose URL names a different repository than the \
                     manifest declares were excluded from this observation entirely"
                .to_owned(),
            omitted_refs: cross_repository_excluded.to_vec(),
        });
    }
    losses.extend(truncation_losses(capture_totals));
    losses
}

/// A capture that hit a provider page limit surfaces here rather than
/// silently under-reporting — every `totalCount` vs received-node mismatch
/// T2 recorded becomes one visible loss entry. Reviews get their own
/// `reviews_truncated` kind rather than being filed under
/// `threads_truncated`: a review is not a thread, and a consumer filtering
/// on `loss_kind` alone should be able to tell them apart without parsing
/// `detail`. `files_truncated` never appears: the `pr` artifact's `files`
/// array carries no `totalCount` in the `gh --json` shape this adapter
/// reads, so file-level truncation is not *detectable* from `CaptureTotals`
/// at all (see `normalize.rs`'s `CaptureTotals` doc comment) — reporting a
/// detected truncation here would fabricate a detection this adapter
/// cannot perform. That blind spot is instead declared unconditionally as
/// `files_completeness_unverifiable` above, which is a different claim
/// ("we cannot tell") from a truncation loss ("we detected this many were
/// dropped") and must not share a `loss_kind` with one.
fn truncation_losses(totals: &CaptureTotals) -> Vec<ProjectionLoss> {
    let mut losses = Vec::new();
    if totals.reviews_received < totals.reviews_reported_total {
        losses.push(ProjectionLoss {
            loss_kind: "reviews_truncated".to_owned(),
            detail: format!(
                "reviews connection reported {} but the capture received {}",
                totals.reviews_reported_total, totals.reviews_received
            ),
            omitted_refs: vec!["capture:reviews".to_owned()],
        });
    }
    if totals.review_threads_received < totals.review_threads_reported_total {
        losses.push(ProjectionLoss {
            loss_kind: "threads_truncated".to_owned(),
            detail: format!(
                "review_threads connection reported {} but the capture received {}",
                totals.review_threads_reported_total, totals.review_threads_received
            ),
            omitted_refs: vec!["capture:review_threads".to_owned()],
        });
    }
    for thread_totals in &totals.thread_comment_totals {
        if thread_totals.received < thread_totals.reported_total {
            // S10: `omitted_refs` names the thread id *and* its path (when
            // the provider reported one) — a reader of `losses` alone,
            // without cross-referencing `can_skim`'s exclusion by id, must
            // still be able to tell which file this loss is about.
            let mut omitted_refs = vec![thread_totals.thread_id.clone()];
            omitted_refs.extend(thread_totals.path.clone());
            losses.push(ProjectionLoss {
                loss_kind: "threads_truncated".to_owned(),
                detail: format!(
                    "thread {} comments connection reported {} but the capture received {}",
                    thread_totals.thread_id, thread_totals.reported_total, thread_totals.received
                ),
                omitted_refs,
            });
        }
    }
    // S2: the checks artifact's `contexts` connection, previously discarded
    // entirely (`GraphQlContextsConnection` read only `nodes`). Unlike
    // `commits` (the independence trust root, refused outright in
    // `normalize()` on detected truncation), a truncated `contexts`
    // connection is declared here plus the `checks_capture_truncated`
    // residual risk above — proportionate to what this connection actually
    // is: an incomplete check/status set, not a corrupted trust root.
    if totals.contexts_received < totals.contexts_reported_total {
        losses.push(ProjectionLoss {
            loss_kind: "contexts_truncated".to_owned(),
            detail: format!(
                "checks contexts connection reported {} but the capture received {}",
                totals.contexts_reported_total, totals.contexts_received
            ),
            omitted_refs: vec!["capture:checks".to_owned()],
        });
    }
    losses
}

/// The `projection_content_hash` pattern (`memory::validation`): `sha256:`
/// of the record's own canonical serialization with its hash field cleared
/// first.
fn projection_content_hash(projection: &ReviewProjection) -> String {
    let mut cleared = projection.clone();
    cleared.projection_content_hash.clear();
    let bytes = serde_json::to_vec(&cleared).expect("typed review_projection serializes");
    format!("sha256:{}", sha256_hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(changed_file_paths: &[&str]) -> PrObservation {
        PrObservation {
            schema: GITHUB_PR_OBSERVATION_SCHEMA.to_owned(),
            observation_id:
                "github-observation:OWNER/repo#7@bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_owned(),
            repository: "OWNER/repo".to_owned(),
            issues: Vec::new(),
            pr: PrObservationPr {
                number: 7,
                title: "Add a thing".to_owned(),
                url: "https://github.com/OWNER/repo/pull/7".to_owned(),
                state: "OPEN".to_owned(),
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
                sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            },
            head: RefSha {
                git_ref: "feature".to_owned(),
                sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
            },
            liveness: Liveness {
                state: "OPEN".to_owned(),
                mergeable: MergeableState::Unknown,
                merge_state_status: "UNKNOWN".to_owned(),
                merged_at: None,
                closed_at: None,
                merge_commit_sha: None,
            },
            changed_files: changed_file_paths
                .iter()
                .map(|path| ChangedFile {
                    path: (*path).to_owned(),
                    additions: 1,
                    deletions: 0,
                    change_type: "MODIFIED".to_owned(),
                })
                .collect(),
            implementation_actors: ImplementationActors {
                actor_ids: vec!["actor:pr-author".to_owned()],
                logins: vec!["alice".to_owned()],
                derivation: "pr_author_and_commit_authors_and_committers".to_owned(),
            },
            source_record_ids: vec!["github-source:pr:sha256-abc".to_owned()],
            captured_at: "2026-01-01T00:00:00Z".to_owned(),
            provider_fields_unmapped: true,
            normalized_content_hash:
                "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
        }
    }

    fn finding(
        id_seed: &str,
        path: Option<&str>,
        actionable: bool,
        resolved: Option<bool>,
        edited: bool,
        author_id: Option<&str>,
    ) -> ReviewFinding {
        ReviewFinding {
            schema: GITHUB_REVIEW_FINDING_SCHEMA.to_owned(),
            finding_id: format!("finding:{id_seed}"),
            kind: if path.is_some() {
                FindingKind::ThreadComment
            } else {
                FindingKind::ReviewSummary
            },
            author: ReviewFindingAuthor {
                id: author_id.map(str::to_owned),
                login: "reviewbot".to_owned(),
                typename: Some("Bot".to_owned()),
                association: "NONE".to_owned(),
            },
            authored_at: "2026-01-01T02:00:00Z".to_owned(),
            last_edited_at: if edited {
                Some("2026-01-01T03:00:00Z".to_owned())
            } else {
                None
            },
            edited,
            url: format!("https://github.com/OWNER/repo/pull/7#{id_seed}"),
            path: path.map(str::to_owned),
            review_state: None,
            commit_sha: None,
            body_content_hash:
                "sha256:2222222222222222222222222222222222222222222222222222222222222222".to_owned(),
            actionable,
            thread: resolved.map(|is_resolved| ReviewThreadState {
                thread_id: format!("thread:{id_seed}"),
                resolved: is_resolved,
                outdated: false,
                resolved_by: None,
                comment_count: 1,
            }),
            duplicate_count: 1,
            source_record_id: "github-source:review_threads:sha256-abc".to_owned(),
        }
    }

    fn check_run(name: &str, conclusion: Option<&str>) -> CheckEvidence {
        CheckEvidence {
            schema: GITHUB_CHECK_EVIDENCE_SCHEMA.to_owned(),
            check_id: format!("check:head:check_run:{name}"),
            head_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
            kind: CheckKind::CheckRun,
            name: name.to_owned(),
            workflow_name: None,
            status: Some("COMPLETED".to_owned()),
            conclusion: conclusion.map(str::to_owned),
            state: None,
            creator: None,
            details_url: None,
            target_url: None,
            description: None,
            started_at: None,
            completed_at: Some("2026-01-01T03:05:00Z".to_owned()),
            created_at: None,
            evidence_role: "ci_check".to_owned(),
            source_record_id: "github-source:checks:sha256-abc".to_owned(),
        }
    }

    fn status_context(name: &str, state: Option<&str>, description: Option<&str>) -> CheckEvidence {
        CheckEvidence {
            schema: GITHUB_CHECK_EVIDENCE_SCHEMA.to_owned(),
            check_id: format!("check:head:status_context:{name}"),
            head_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
            kind: CheckKind::StatusContext,
            name: name.to_owned(),
            workflow_name: None,
            status: None,
            conclusion: None,
            state: state.map(str::to_owned),
            creator: None,
            details_url: None,
            target_url: None,
            description: description.map(str::to_owned),
            started_at: None,
            completed_at: None,
            created_at: Some("2026-01-01T03:00:00Z".to_owned()),
            evidence_role: "ci_check".to_owned(),
            source_record_id: "github-source:checks:sha256-abc".to_owned(),
        }
    }

    fn independence(
        unresolved: &[&str],
        resolved: &[&str],
        classifications: Vec<Classification>,
        approvals: &[&str],
        require_independent_review: bool,
    ) -> ReviewIndependence {
        let satisfying_finding_ids: Vec<String> = approvals
            .iter()
            .map(|seed| format!("finding:{seed}"))
            .collect();
        let satisfied = !satisfying_finding_ids.is_empty() || !require_independent_review;
        ReviewIndependence {
            schema: GITHUB_REVIEW_INDEPENDENCE_SCHEMA.to_owned(),
            pr_observation_hash:
                "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
            implementation_actor_ids: vec!["actor:pr-author".to_owned()],
            implementation_actor_logins: vec!["alice".to_owned()],
            classifications,
            unresolved_actionable_finding_ids: unresolved
                .iter()
                .map(|seed| format!("finding:{seed}"))
                .collect(),
            resolved_actionable_finding_ids: resolved
                .iter()
                .map(|seed| format!("finding:{seed}"))
                .collect(),
            independent_human_approvals: satisfying_finding_ids.clone(),
            excluded_approvals: Vec::new(),
            policy: IndependencePolicy {
                require_independent_review,
                satisfied,
                satisfying_finding_ids,
            },
            independence_proven: false,
            findings: vec![IndependenceFinding {
                code: "independent_minds_not_observable".to_owned(),
                detail: "different actor ids do not prove independent minds or undeclared \
                          information isolation"
                    .to_owned(),
            }],
        }
    }

    fn no_truncation() -> CaptureTotals {
        CaptureTotals {
            reviews_reported_total: 0,
            reviews_received: 0,
            review_threads_reported_total: 0,
            review_threads_received: 0,
            thread_comment_totals: Vec::new(),
            contexts_reported_total: 0,
            contexts_received: 0,
        }
    }

    #[test]
    fn unresolved_is_must_resolved_is_should_untouched_file_is_can_skim() {
        let obs = observation(&["a.rs", "b.rs", "c.rs"]);
        let unresolved = finding(
            "unresolved",
            Some("a.rs"),
            true,
            Some(false),
            false,
            Some("actor:bot-1"),
        );
        let resolved = finding(
            "resolved",
            Some("b.rs"),
            true,
            Some(true),
            false,
            Some("actor:bot-1"),
        );
        let findings = vec![unresolved.clone(), resolved.clone()];
        let classifications = vec![
            Classification {
                subject_id: unresolved.finding_id.clone(),
                evidence_role: EvidenceRole::AutomatedBot,
                basis: ClassificationBasis::ProviderBotDiscriminator,
            },
            Classification {
                subject_id: resolved.finding_id.clone(),
                evidence_role: EvidenceRole::AutomatedBot,
                basis: ClassificationBasis::ProviderBotDiscriminator,
            },
        ];
        let indep = independence(&["unresolved"], &["resolved"], classifications, &[], false);
        let projection = project_review(&obs, &[], &findings, &indep, &no_truncation(), &[], None);

        assert!(projection
            .must_review
            .iter()
            .any(|item| item.subject_ids.contains(&unresolved.finding_id)));
        assert!(projection
            .blocking_findings
            .iter()
            .any(|f| f.finding_id == unresolved.finding_id));
        assert!(!projection
            .must_review
            .iter()
            .any(|item| item.subject_ids.contains(&resolved.finding_id)));

        assert!(projection
            .should_review
            .iter()
            .any(|item| item.subject_ids.contains(&resolved.finding_id)));
        assert!(projection
            .non_blocking_findings
            .iter()
            .any(|f| f.finding_id == resolved.finding_id));

        assert!(projection
            .can_skim
            .iter()
            .any(|item| item.path.as_deref() == Some("c.rs")));
        assert!(!projection.can_skim.iter().any(
            |item| item.path.as_deref() == Some("a.rs") || item.path.as_deref() == Some("b.rs")
        ));
    }

    #[test]
    fn losses_are_never_empty_and_truncation_is_visible() {
        let obs = observation(&["a.rs"]);
        let indep = independence(&[], &[], Vec::new(), &[], false);

        let clean = project_review(&obs, &[], &[], &indep, &no_truncation(), &[], None);
        assert!(!clean.losses.is_empty());
        assert!(clean
            .losses
            .iter()
            .any(|loss| loss.loss_kind == "bodies_hashed"));
        assert!(clean
            .losses
            .iter()
            .any(|loss| loss.loss_kind == "provider_fields_unmapped"));
        assert!(!clean
            .losses
            .iter()
            .any(|loss| loss.loss_kind == "threads_truncated"));
        assert!(!clean
            .losses
            .iter()
            .any(|loss| loss.loss_kind == "reviews_truncated"));

        let truncated_totals = CaptureTotals {
            reviews_reported_total: 6,
            reviews_received: 5,
            review_threads_reported_total: 3,
            review_threads_received: 2,
            thread_comment_totals: Vec::new(),
            contexts_reported_total: 0,
            contexts_received: 0,
        };
        let truncated = project_review(&obs, &[], &[], &indep, &truncated_totals, &[], None);
        assert!(truncated
            .losses
            .iter()
            .any(|loss| loss.loss_kind == "threads_truncated"
                && loss.detail.contains("review_threads")));
        // Reviews and threads are distinguishable by loss_kind alone, not
        // only by parsing detail — a review truncation is never filed
        // under the thread loss_kind or vice versa.
        assert!(truncated
            .losses
            .iter()
            .any(|loss| loss.loss_kind == "reviews_truncated" && loss.detail.contains("reviews")));
        assert!(!truncated
            .losses
            .iter()
            .any(|loss| loss.loss_kind == "threads_truncated"
                && loss.detail.contains("reviews connection")));
    }

    /// S2: a truncated `checks` `contexts` connection (previously silently
    /// discarded — `GraphQlContextsConnection` read only `nodes`, never
    /// `totalCount`) surfaces as its own `contexts_truncated` loss plus a
    /// `checks_capture_truncated` residual risk — proportionate to
    /// `contexts` not being the independence trust root (`commits` is,
    /// which `normalize()` refuses outright on detected truncation instead).
    #[test]
    fn truncated_checks_contexts_is_a_declared_loss_and_residual_risk() {
        let obs = observation(&["a.rs"]);
        let indep = independence(&[], &[], Vec::new(), &[], false);

        let clean = project_review(&obs, &[], &[], &indep, &no_truncation(), &[], None);
        assert!(!clean
            .losses
            .iter()
            .any(|loss| loss.loss_kind == "contexts_truncated"));
        assert!(!clean
            .residual_risks
            .iter()
            .any(|risk| risk.code == "checks_capture_truncated"));

        let truncated_totals = CaptureTotals {
            reviews_reported_total: 0,
            reviews_received: 0,
            review_threads_reported_total: 0,
            review_threads_received: 0,
            thread_comment_totals: Vec::new(),
            contexts_reported_total: 3,
            contexts_received: 2,
        };
        let truncated = project_review(&obs, &[], &[], &indep, &truncated_totals, &[], None);
        assert!(truncated
            .losses
            .iter()
            .any(|loss| loss.loss_kind == "contexts_truncated"
                && loss.detail.contains('3')
                && loss.detail.contains('2')));
        assert!(truncated
            .residual_risks
            .iter()
            .any(|risk| risk.code == "checks_capture_truncated"));
    }

    #[test]
    fn files_completeness_is_always_declared_unverifiable_regardless_of_actual_completeness() {
        // The `files` capture carries no total count, so this loss cannot
        // depend on detection — it is emitted whether or not the changed
        // file list happens to be complete (here it plainly is: one file,
        // no truncation anywhere in capture_totals).
        let obs = observation(&["a.rs"]);
        let indep = independence(&[], &[], Vec::new(), &[], false);
        let projection = project_review(&obs, &[], &[], &indep, &no_truncation(), &[], None);

        assert!(projection
            .losses
            .iter()
            .any(|loss| loss.loss_kind == "files_completeness_unverifiable"));
    }

    #[test]
    fn skipped_check_is_inconclusive_never_success_or_failure() {
        let obs = observation(&["a.rs"]);
        let check = check_run("quality", Some("SKIPPED"));
        let indep = independence(
            &[],
            &[],
            vec![Classification {
                subject_id: check.check_id.clone(),
                evidence_role: EvidenceRole::CiCheck,
                basis: ClassificationBasis::CheckObservation,
            }],
            &[],
            false,
        );
        let projection = project_review(
            &obs,
            std::slice::from_ref(&check),
            &[],
            &indep,
            &no_truncation(),
            &[],
            None,
        );

        assert!(projection.inconclusive_checks.contains(&check.check_id));
        assert!(!projection.failed_checks.contains(&check.check_id));
        assert!(projection
            .should_review
            .iter()
            .any(|item| item.subject_ids.contains(&check.check_id)));
        assert!(!projection
            .must_review
            .iter()
            .any(|item| item.subject_ids.contains(&check.check_id)));
        assert!(projection
            .residual_risks
            .iter()
            .any(|risk| risk.code == "checks_inconclusive"));
    }

    #[test]
    fn status_context_description_surfaces_even_on_success() {
        // The pilot's own case: CodeRabbit's StatusContext is `state:
        // SUCCESS` with description "Review rate limited" — a clean
        // projection here must still say that.
        let obs = observation(&["a.rs"]);
        let status = status_context("CodeRabbit", Some("SUCCESS"), Some("Review rate limited"));
        let indep = independence(
            &[],
            &[],
            vec![Classification {
                subject_id: status.check_id.clone(),
                evidence_role: EvidenceRole::CiCheck,
                basis: ClassificationBasis::CheckObservation,
            }],
            &[],
            false,
        );
        let projection = project_review(
            &obs,
            std::slice::from_ref(&status),
            &[],
            &indep,
            &no_truncation(),
            &[],
            None,
        );

        assert!(!projection.failed_checks.contains(&status.check_id));
        assert!(!projection.inconclusive_checks.contains(&status.check_id));
        assert!(projection
            .residual_risks
            .iter()
            .any(|risk| risk.code == "status_context_description"
                && risk.detail.contains("Review rate limited")));
    }

    #[test]
    fn projection_content_hash_is_deterministic() {
        let obs = observation(&["a.rs"]);
        let check = check_run("quality", Some("SUCCESS"));
        let finding_record = finding(
            "f1",
            Some("a.rs"),
            true,
            Some(false),
            false,
            Some("actor:bot-1"),
        );
        let indep = independence(
            &["f1"],
            &[],
            vec![
                Classification {
                    subject_id: finding_record.finding_id.clone(),
                    evidence_role: EvidenceRole::AutomatedBot,
                    basis: ClassificationBasis::ProviderBotDiscriminator,
                },
                Classification {
                    subject_id: check.check_id.clone(),
                    evidence_role: EvidenceRole::CiCheck,
                    basis: ClassificationBasis::CheckObservation,
                },
            ],
            &[],
            false,
        );

        let first = project_review(
            &obs,
            std::slice::from_ref(&check),
            std::slice::from_ref(&finding_record),
            &indep,
            &no_truncation(),
            &[],
            None,
        );
        let second = project_review(
            &obs,
            std::slice::from_ref(&check),
            std::slice::from_ref(&finding_record),
            &indep,
            &no_truncation(),
            &[],
            None,
        );
        assert_eq!(
            first.projection_content_hash,
            second.projection_content_hash
        );
        assert_eq!(first, second);
    }

    #[test]
    fn unmet_independent_review_policy_blocks_only_when_flag_set() {
        let obs = observation(&["a.rs"]);

        let indep_required = independence(&[], &[], Vec::new(), &[], true);
        let projection_required =
            project_review(&obs, &[], &[], &indep_required, &no_truncation(), &[], None);
        assert!(projection_required
            .blocking_findings
            .iter()
            .any(|f| f.finding_id.starts_with("independent_review_policy:")));
        assert!(projection_required
            .residual_risks
            .iter()
            .any(|risk| risk.code == "no_independent_human_approval"));

        let indep_not_required = independence(&[], &[], Vec::new(), &[], false);
        let projection_not_required = project_review(
            &obs,
            &[],
            &[],
            &indep_not_required,
            &no_truncation(),
            &[],
            None,
        );
        assert!(!projection_not_required
            .blocking_findings
            .iter()
            .any(|f| f.finding_id.starts_with("independent_review_policy:")));
        assert!(projection_not_required
            .residual_risks
            .iter()
            .any(|risk| risk.code == "no_independent_human_approval"));
    }

    #[test]
    fn stale_head_refresh_is_projected_as_a_blocking_must_review_subject() {
        let obs = observation(&["a.rs"]);
        let indep = independence(&[], &[], Vec::new(), &[], false);
        let refresh = RefreshResult {
            schema: GITHUB_REFRESH_RESULT_SCHEMA.to_owned(),
            previous_observation_hash:
                "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
            previous_head_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
            previous_base_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            observed_head_sha: "cccccccccccccccccccccccccccccccccccccccc".to_owned(),
            observed_base_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            disposition: RefreshDisposition::StaleHead,
            review_basis_moved: false,
            observation_changes: Vec::new(),
            refreshed_observation_hash: None,
        };
        let projection = project_review(
            &obs,
            &[],
            &[],
            &indep,
            &no_truncation(),
            &[],
            Some(&refresh),
        );

        assert!(projection
            .must_review
            .iter()
            .any(|item| item.reason.contains("stale")));
        assert!(projection
            .blocking_findings
            .iter()
            .any(|f| f.finding_id.starts_with("stale_head_refresh:")));

        let no_refresh = project_review(&obs, &[], &[], &indep, &no_truncation(), &[], None);
        assert!(no_refresh.must_review.is_empty());
    }
}
