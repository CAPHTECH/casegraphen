//! The single implementation of the independence decision rule (design §6).
//!
//! Three questions, three functions, in the order a caller must use them:
//!
//! 1. [`implementation_actor_ids`] — the node-id set a subject can never
//!    escape by rename, read off the already-computed
//!    `pr_observation.implementation_actors` field (`normalize.rs` owns the
//!    *construction* of that set from the captured `pr` + `commits`
//!    artifacts; this function only gives the classifier the `BTreeSet` type
//!    it needs, so the membership rule itself still has exactly one place it
//!    is decided).
//! 2. [`classify_evidence_role`] — the closed, ordered, five-arm classifier.
//!    Every subject in a capture is exactly one of `self_review`,
//!    `automated_bot`, `ci_check`, `independent_human_candidate`, or
//!    `unattributed`. The implementation-actor arm precedes every
//!    attestation arm, so an implementation actor can never reach
//!    `independent_human_candidate` no matter what the provider attests
//!    about it.
//! 3. [`evaluate_independence`] — counts only `independent_human_candidate`
//!    findings with verbatim `review_state == "APPROVED"` and
//!    `commit_sha == head.sha`. There is no absent-binding fallback: a null
//!    or older-commit approval is excluded and the exclusion is recorded,
//!    never silently dropped and never silently credited.
//!
//! `evaluate_independence` never sets `independence_proven`; the record type
//! cannot express it (schema `const: false`, mirrored here as a hardcoded
//! `false`), and the `independent_minds_not_observable` finding — the exact
//! code string and detail `verification_policy.rs` already uses — is always
//! attached. A candidate is a candidate; this module does not, and cannot,
//! prove independent minds.

use super::model::{
    CheckEvidence, Classification, ClassificationBasis, EvidenceRole, ExcludedApproval,
    IndependenceFinding, IndependencePolicy, PrObservation, ReviewFinding, ReviewIndependence,
    GITHUB_REVIEW_INDEPENDENCE_SCHEMA,
};
use std::collections::{BTreeMap, BTreeSet};

/// Reads the already-computed implementation actor-id set off a
/// `pr_observation`. The set's *construction* — pr author id ∪ commit
/// author ids ∪ committer ids, all node ids, never logins — lives in
/// `normalize.rs` and only there; this function does not re-derive
/// membership, so the two cannot disagree about who is an implementation
/// actor.
pub fn implementation_actor_ids(observation: &PrObservation) -> BTreeSet<String> {
    observation
        .implementation_actors
        .actor_ids
        .iter()
        .cloned()
        .collect()
}

/// What is being classified: a check/status observation (always
/// `ci_check`, regardless of who created it — arm 1 does not look at a
/// creator at all), or a review finding's author, given by its GitHub node
/// id and GraphQL `__typename`, both independently optional exactly as
/// `ReviewFindingAuthor` (`model.rs`) carries them.
#[derive(Clone, Copy, Debug)]
pub enum EvidenceSubject<'a> {
    Check,
    Author {
        id: Option<&'a str>,
        typename: Option<&'a str>,
    },
}

/// The closed, ordered five-arm classifier (design §6, arm order exactly as
/// written there):
///
/// ```text
/// 1. subject is a check_run or status_context       -> ci_check
/// 2. author.id in implementation_actor_ids           -> self_review
/// 3. bot-attested by the ordered list below           -> automated_bot
/// 4. typename == "User" and not bot-attested          -> independent_human_candidate
/// 5. no typename and not bot-attested                 -> unattributed
/// ```
///
/// Bot attestation (arm 3) is itself an ordered, closed list — never a name
/// heuristic:
///
/// 1. the GraphQL Actor discriminator `__typename` is present and is not
///    `"User"` (`Bot`, `Organization`, `Mannequin`,
///    `EnterpriseUserAccount`, … all fail closed into the non-human role);
/// 2. the provider-issued `BOT_` node-id prefix — this outranks a `User`
///    typename, because the real PR-101 corpus attests the same node id
///    `BOT_kgDOCCSy2w` as `Bot` on its review/comment authorship and as
///    `User` in `resolvedBy` within one capture; only an id-keyed rule
///    resolves that without contradiction;
/// 3. id equality with an actor already bot-attested elsewhere in the same
///    capture (`bot_attested_ids`, computed once across every finding
///    author and check creator by [`evaluate_independence`]) — the sticky
///    case: a `User` typename on *this* occurrence never overrides an
///    attestation established by another occurrence of the same id.
///
/// Returns both the role and which rule produced it in one evaluation, so
/// `Classification.evidence_role` and `Classification.basis` can never
/// disagree about which arm fired.
pub fn classify_evidence_role(
    subject: EvidenceSubject<'_>,
    implementation_actor_ids: &BTreeSet<String>,
    bot_attested_ids: &BTreeSet<String>,
) -> (EvidenceRole, ClassificationBasis) {
    let (id, typename) = match subject {
        EvidenceSubject::Check => {
            return (EvidenceRole::CiCheck, ClassificationBasis::CheckObservation);
        }
        EvidenceSubject::Author { id, typename } => (id, typename),
    };

    // Arm 2: an implementation actor can never reach candidacy, no matter
    // what the provider attests about it — checked before any attestation
    // arm.
    if let Some(id) = id {
        if implementation_actor_ids.contains(id) {
            return (
                EvidenceRole::SelfReview,
                ClassificationBasis::AuthorInImplementationActorSet,
            );
        }
    }

    // Arm 3, rule 1: any non-"User" discriminator fails closed to
    // automated_bot, independent of id.
    if let Some(typename) = typename {
        if typename != "User" {
            return (
                EvidenceRole::AutomatedBot,
                ClassificationBasis::ProviderBotDiscriminator,
            );
        }
    }
    if let Some(id) = id {
        // Arm 3, rule 2: the provider-issued id prefix outranks a `User`
        // typename on this same occurrence.
        if id.starts_with("BOT_") {
            return (
                EvidenceRole::AutomatedBot,
                ClassificationBasis::ProviderBotIdPrefix,
            );
        }
        // Arm 3, rule 3: sticky bot attestation via id equality with an
        // actor already bot-attested elsewhere in this capture.
        if bot_attested_ids.contains(id) {
            return (
                EvidenceRole::AutomatedBot,
                ClassificationBasis::ProviderBotIdEquality,
            );
        }
    }

    // Arm 4: a provider-attested human who is neither an implementation
    // actor nor bot-attested by any rule above.
    if typename == Some("User") {
        return (
            EvidenceRole::IndependentHumanCandidate,
            ClassificationBasis::ProviderUserDiscriminator,
        );
    }

    // Arm 5: absence is recorded, not guessed. Fails closed — cannot
    // satisfy an independent-review policy.
    (
        EvidenceRole::Unattributed,
        ClassificationBasis::AttestationAbsent,
    )
}

/// Scans every finding author and check creator in the capture once, and
/// returns the set of node ids that are bot-attested by rule 1
/// (`__typename` present and not `"User"`, at *any* occurrence of that id)
/// or rule 2 (the `BOT_` id prefix, which needs no aggregation since it is
/// a property of the id string itself). This is exactly the set
/// [`classify_evidence_role`]'s arm-3 rule 3 consults; it is built once per
/// evaluation, not per subject, so every subject sharing an id sees the
/// same sticky attestation.
fn compute_bot_attested_ids(
    findings: &[ReviewFinding],
    checks: &[CheckEvidence],
) -> BTreeSet<String> {
    let mut bot_ids = BTreeSet::new();
    let mut consider = |id: Option<&str>, typename: Option<&str>| {
        if let Some(id) = id {
            if id.starts_with("BOT_") {
                bot_ids.insert(id.to_owned());
            }
            if let Some(typename) = typename {
                if typename != "User" {
                    bot_ids.insert(id.to_owned());
                }
            }
        }
    };
    for finding in findings {
        consider(
            finding.author.id.as_deref(),
            finding.author.typename.as_deref(),
        );
    }
    for check in checks {
        if let Some(creator) = &check.creator {
            consider(Some(creator.id.as_str()), Some(creator.typename.as_str()));
        }
    }
    bot_ids
}

/// Classifies every check and finding in one capture and evaluates whether
/// the result can satisfy an independent-review policy. Counts **only**
/// `independent_human_candidate` findings with verbatim
/// `review_state == "APPROVED"` and `commit_sha == observation.head.sha` —
/// no absent-binding fallback (design §3.5): a null or older `commit_sha`
/// lands in `excluded_approvals` with reason
/// `approval_not_bound_to_observed_head`, visibly recorded rather than
/// silently dropped or silently credited.
///
/// `independence_proven` is hardcoded `false` and the
/// `independent_minds_not_observable` finding is always attached, verbatim
/// the same code string and detail `verification_policy.rs` uses for the
/// identical stance.
pub fn evaluate_independence(
    observation: &PrObservation,
    findings: &[ReviewFinding],
    checks: &[CheckEvidence],
    require_independent_review: bool,
) -> ReviewIndependence {
    let actor_ids = implementation_actor_ids(observation);
    let bot_attested_ids = compute_bot_attested_ids(findings, checks);

    let mut classifications = Vec::with_capacity(findings.len() + checks.len());
    let mut roles_by_finding_id: BTreeMap<&str, EvidenceRole> = BTreeMap::new();

    for check in checks {
        classifications.push(Classification {
            subject_id: check.check_id.clone(),
            evidence_role: EvidenceRole::CiCheck,
            basis: ClassificationBasis::CheckObservation,
        });
    }
    for finding in findings {
        let (role, basis) = classify_evidence_role(
            EvidenceSubject::Author {
                id: finding.author.id.as_deref(),
                typename: finding.author.typename.as_deref(),
            },
            &actor_ids,
            &bot_attested_ids,
        );
        roles_by_finding_id.insert(finding.finding_id.as_str(), role);
        classifications.push(Classification {
            subject_id: finding.finding_id.clone(),
            evidence_role: role,
            basis,
        });
    }
    classifications.sort_by(|left, right| left.subject_id.cmp(&right.subject_id));

    let mut unresolved_actionable_finding_ids = Vec::new();
    let mut resolved_actionable_finding_ids = Vec::new();
    for finding in findings.iter().filter(|finding| finding.actionable) {
        match &finding.thread {
            Some(thread) if thread.resolved => {
                resolved_actionable_finding_ids.push(finding.finding_id.clone());
            }
            _ => unresolved_actionable_finding_ids.push(finding.finding_id.clone()),
        }
    }
    unresolved_actionable_finding_ids.sort();
    resolved_actionable_finding_ids.sort();

    let mut independent_human_approvals = Vec::new();
    let mut excluded_approvals = Vec::new();
    for finding in findings {
        if finding.review_state.as_deref() != Some("APPROVED") {
            continue;
        }
        if roles_by_finding_id.get(finding.finding_id.as_str())
            != Some(&EvidenceRole::IndependentHumanCandidate)
        {
            continue;
        }
        if finding.commit_sha.as_deref() == Some(observation.head.sha.as_str()) {
            independent_human_approvals.push(finding.finding_id.clone());
        } else {
            excluded_approvals.push(ExcludedApproval {
                finding_id: finding.finding_id.clone(),
                reason: "approval_not_bound_to_observed_head".to_owned(),
            });
        }
    }
    independent_human_approvals.sort();
    excluded_approvals.sort_by(|left, right| left.finding_id.cmp(&right.finding_id));

    let satisfied = !require_independent_review || !independent_human_approvals.is_empty();
    let satisfying_finding_ids = if satisfied {
        independent_human_approvals.clone()
    } else {
        Vec::new()
    };

    ReviewIndependence {
        schema: GITHUB_REVIEW_INDEPENDENCE_SCHEMA.to_owned(),
        pr_observation_hash: observation.normalized_content_hash.clone(),
        implementation_actor_ids: observation.implementation_actors.actor_ids.clone(),
        implementation_actor_logins: observation.implementation_actors.logins.clone(),
        classifications,
        unresolved_actionable_finding_ids,
        resolved_actionable_finding_ids,
        independent_human_approvals,
        excluded_approvals,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_evidence::model::{
        Actor, CheckKind, FindingKind, ImplementationActors, Liveness, MergeableState, PrAuthor,
        PrObservationPr, RefSha, ReviewFindingAuthor, ReviewThreadState,
        GITHUB_CHECK_EVIDENCE_SCHEMA, GITHUB_PR_OBSERVATION_SCHEMA, GITHUB_REVIEW_FINDING_SCHEMA,
    };

    const PR_AUTHOR_ID: &str = "MDQ6VXNlcjc5MDUxMQ==";
    const HEAD_SHA: &str = "c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b";
    const BASE_SHA: &str = "947f347f219a60775bcf71b226ce778cc8ea21f4";
    /// The real PR-101 corpus's older, not-bound-to-head commit — the review
    /// at this commit is the actual "approval at an older commit" case
    /// (design §10.1's exclusion fixture), replayed here with `review_state`
    /// swapped to `APPROVED` and a `User` author per the T3 task brief.
    const OLDER_SHA: &str = "5403673f13b45d8deb0f4be62f50390172071bb0";
    /// The real PR-101 corpus's bot actor id — attested `Bot` on its own
    /// reviews/comments and, contradictorily, `User`-typed in `resolvedBy`
    /// (design §6); used to exercise the `BOT_`-prefix arm exactly as the
    /// live data forces it to fire.
    const BOT_ID: &str = "BOT_kgDOCCSy2w";

    fn observation_with_actors(actor_ids: &[&str], logins: &[&str]) -> PrObservation {
        PrObservation {
            schema: GITHUB_PR_OBSERVATION_SCHEMA.to_owned(),
            observation_id: format!("github-observation:OWNER/repo#101@{HEAD_SHA}"),
            repository: "OWNER/repo".to_owned(),
            issues: Vec::new(),
            pr: PrObservationPr {
                number: 101,
                title: "Add a thing".to_owned(),
                url: "https://github.com/OWNER/repo/pull/101".to_owned(),
                state: "MERGED".to_owned(),
                author: PrAuthor {
                    id: PR_AUTHOR_ID.to_owned(),
                    login: "rizumita".to_owned(),
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
                sha: HEAD_SHA.to_owned(),
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
                actor_ids: actor_ids.iter().map(|id| (*id).to_owned()).collect(),
                logins: logins.iter().map(|login| (*login).to_owned()).collect(),
                derivation: "pr_author_and_commit_authors_and_committers".to_owned(),
            },
            source_record_ids: Vec::new(),
            captured_at: "2026-01-01T00:00:00Z".to_owned(),
            provider_fields_unmapped: true,
            normalized_content_hash: "sha256:test-observation".to_owned(),
        }
    }

    fn observation() -> PrObservation {
        observation_with_actors(&[PR_AUTHOR_ID], &["rizumita"])
    }

    #[allow(clippy::too_many_arguments)]
    fn review(
        finding_id: &str,
        author_id: Option<&str>,
        typename: Option<&str>,
        login: &str,
        association: &str,
        state: &str,
        commit_sha: Option<&str>,
    ) -> ReviewFinding {
        ReviewFinding {
            schema: GITHUB_REVIEW_FINDING_SCHEMA.to_owned(),
            finding_id: finding_id.to_owned(),
            kind: FindingKind::ReviewSummary,
            author: ReviewFindingAuthor {
                id: author_id.map(str::to_owned),
                login: login.to_owned(),
                typename: typename.map(str::to_owned),
                association: association.to_owned(),
            },
            authored_at: "2026-01-01T02:00:00Z".to_owned(),
            last_edited_at: None,
            edited: false,
            url: format!("https://github.com/OWNER/repo/pull/101#{finding_id}"),
            path: None,
            review_state: Some(state.to_owned()),
            commit_sha: commit_sha.map(str::to_owned),
            body_content_hash:
                "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
            actionable: false,
            thread: None,
            duplicate_count: 1,
            source_record_id: "github-source:reviews:sha256-abc".to_owned(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn thread_comment(
        finding_id: &str,
        author_id: Option<&str>,
        typename: Option<&str>,
        login: &str,
        thread_id: &str,
        resolved: bool,
        actionable: bool,
    ) -> ReviewFinding {
        ReviewFinding {
            schema: GITHUB_REVIEW_FINDING_SCHEMA.to_owned(),
            finding_id: finding_id.to_owned(),
            kind: FindingKind::ThreadComment,
            author: ReviewFindingAuthor {
                id: author_id.map(str::to_owned),
                login: login.to_owned(),
                typename: typename.map(str::to_owned),
                association: "NONE".to_owned(),
            },
            authored_at: "2026-01-01T02:30:00Z".to_owned(),
            last_edited_at: None,
            edited: false,
            url: format!("https://github.com/OWNER/repo/pull/101#{finding_id}"),
            path: Some("a.rs".to_owned()),
            review_state: None,
            commit_sha: None,
            body_content_hash:
                "sha256:2222222222222222222222222222222222222222222222222222222222222222".to_owned(),
            actionable,
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

    fn check(check_id: &str, creator: Option<Actor>) -> CheckEvidence {
        CheckEvidence {
            schema: GITHUB_CHECK_EVIDENCE_SCHEMA.to_owned(),
            check_id: check_id.to_owned(),
            head_sha: HEAD_SHA.to_owned(),
            kind: CheckKind::CheckRun,
            name: "quality".to_owned(),
            workflow_name: None,
            status: Some("COMPLETED".to_owned()),
            conclusion: Some("SUCCESS".to_owned()),
            state: None,
            creator,
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

    fn role_of<'a>(independence: &'a ReviewIndependence, subject_id: &str) -> &'a EvidenceRole {
        &independence
            .classifications
            .iter()
            .find(|classification| classification.subject_id == subject_id)
            .unwrap_or_else(|| panic!("no classification for {subject_id}"))
            .evidence_role
    }

    #[test]
    fn pr_author_approval_is_self_review() {
        let observation = observation();
        let findings = vec![review(
            "finding:1",
            Some(PR_AUTHOR_ID),
            Some("User"),
            "rizumita",
            "MEMBER",
            "APPROVED",
            Some(HEAD_SHA),
        )];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(*role_of(&result, "finding:1"), EvidenceRole::SelfReview);
        assert!(result.independent_human_approvals.is_empty());
        assert!(!result.policy.satisfied);
    }

    #[test]
    fn commit_author_under_a_different_login_is_still_self_review() {
        // Actor substitution / rename case: the id is in the implementation
        // actor set (as if this actor authored a commit under a different
        // login than the one on this review), so arm 2 must still fire on
        // id equality alone.
        let observation = observation_with_actors(
            &[PR_AUTHOR_ID, "actor:committer"],
            &["rizumita", "old-login"],
        );
        let findings = vec![review(
            "finding:1",
            Some("actor:committer"),
            Some("User"),
            "renamed-login",
            "NONE",
            "APPROVED",
            Some(HEAD_SHA),
        )];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(*role_of(&result, "finding:1"), EvidenceRole::SelfReview);
        assert!(result.independent_human_approvals.is_empty());
    }

    #[test]
    fn bot_typename_approved_is_automated_bot() {
        let observation = observation();
        let findings = vec![review(
            "finding:1",
            Some("actor:reviewbot"),
            Some("Bot"),
            "reviewbot",
            "NONE",
            "APPROVED",
            Some(HEAD_SHA),
        )];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(*role_of(&result, "finding:1"), EvidenceRole::AutomatedBot);
        let classification = result
            .classifications
            .iter()
            .find(|classification| classification.subject_id == "finding:1")
            .unwrap();
        assert_eq!(
            classification.basis,
            ClassificationBasis::ProviderBotDiscriminator
        );
        assert!(result.independent_human_approvals.is_empty());
    }

    #[test]
    fn organization_typename_is_automated_bot_fail_closed_non_user_arm() {
        let observation = observation();
        let findings = vec![review(
            "finding:1",
            Some("actor:some-org"),
            Some("Organization"),
            "some-org",
            "NONE",
            "APPROVED",
            Some(HEAD_SHA),
        )];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(*role_of(&result, "finding:1"), EvidenceRole::AutomatedBot);
        assert!(result.independent_human_approvals.is_empty());
    }

    #[test]
    fn outside_user_commented_is_candidate_but_does_not_satisfy() {
        let observation = observation();
        let findings = vec![review(
            "finding:1",
            Some("actor:outside"),
            Some("User"),
            "carol",
            "NONE",
            "COMMENTED",
            Some(HEAD_SHA),
        )];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(
            *role_of(&result, "finding:1"),
            EvidenceRole::IndependentHumanCandidate
        );
        assert!(result.independent_human_approvals.is_empty());
        assert!(result.excluded_approvals.is_empty());
        assert!(!result.policy.satisfied);
    }

    #[test]
    fn outside_user_approved_at_head_satisfies() {
        let observation = observation();
        let findings = vec![review(
            "finding:1",
            Some("actor:outside"),
            Some("User"),
            "carol",
            "NONE",
            "APPROVED",
            Some(HEAD_SHA),
        )];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(
            *role_of(&result, "finding:1"),
            EvidenceRole::IndependentHumanCandidate
        );
        assert_eq!(
            result.independent_human_approvals,
            vec!["finding:1".to_owned()]
        );
        assert!(result.policy.satisfied);
        assert_eq!(
            result.policy.satisfying_finding_ids,
            vec!["finding:1".to_owned()]
        );
    }

    #[test]
    fn real_pilot_older_head_review_swapped_to_approved_is_excluded() {
        // The real PR-101 corpus's review at the older commit
        // (pullrequestreview-4872254337), replayed with review_state
        // swapped to APPROVED and a User author, per the T3 brief — proves
        // the no-absent-binding-fallback exclusion on genuine corpus shape
        // rather than an invented commit sha.
        let observation = observation();
        let findings = vec![review(
            "finding:older-head",
            Some("actor:outside"),
            Some("User"),
            "carol",
            "NONE",
            "APPROVED",
            Some(OLDER_SHA),
        )];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(
            *role_of(&result, "finding:older-head"),
            EvidenceRole::IndependentHumanCandidate
        );
        assert!(result.independent_human_approvals.is_empty());
        assert_eq!(
            result.excluded_approvals,
            vec![ExcludedApproval {
                finding_id: "finding:older-head".to_owned(),
                reason: "approval_not_bound_to_observed_head".to_owned(),
            }]
        );
        assert!(!result.policy.satisfied);
    }

    #[test]
    fn null_commit_sha_approval_is_excluded() {
        let observation = observation();
        let findings = vec![review(
            "finding:1",
            Some("actor:outside"),
            Some("User"),
            "carol",
            "NONE",
            "APPROVED",
            None,
        )];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert!(result.independent_human_approvals.is_empty());
        assert_eq!(
            result.excluded_approvals[0].reason,
            "approval_not_bound_to_observed_head"
        );
    }

    #[test]
    fn author_association_is_never_read() {
        let observation = observation();
        let findings = vec![
            review(
                "finding:member",
                Some("actor:outside"),
                Some("User"),
                "carol",
                "MEMBER",
                "APPROVED",
                Some(HEAD_SHA),
            ),
            review(
                "finding:none",
                Some("actor:outside-2"),
                Some("User"),
                "dave",
                "NONE",
                "APPROVED",
                Some(HEAD_SHA),
            ),
        ];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(
            *role_of(&result, "finding:member"),
            EvidenceRole::IndependentHumanCandidate
        );
        assert_eq!(
            *role_of(&result, "finding:none"),
            EvidenceRole::IndependentHumanCandidate
        );
        let mut approvals = result.independent_human_approvals.clone();
        approvals.sort();
        assert_eq!(
            approvals,
            vec!["finding:member".to_owned(), "finding:none".to_owned()]
        );
    }

    #[test]
    fn login_only_author_approved_is_unattributed_and_satisfies_nothing() {
        let observation = observation();
        let findings = vec![review(
            "finding:1",
            None,
            None,
            "deleted-account",
            "NONE",
            "APPROVED",
            Some(HEAD_SHA),
        )];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(*role_of(&result, "finding:1"), EvidenceRole::Unattributed);
        assert!(result.independent_human_approvals.is_empty());
        assert!(result.excluded_approvals.is_empty());
    }

    #[test]
    fn bot_id_prefix_outranks_user_typename_the_resolver_shape() {
        // The corpus's own contradiction: the same node id is attested
        // `Bot` on its comments and `User` in `resolvedBy`. A finding
        // authored under a User-typed occurrence of that same BOT_-prefixed
        // id must still classify automated_bot via the id prefix, not the
        // typename — proving the prefix rule outranks a User discriminator
        // on the very shape that makes this load-bearing.
        let observation = observation();
        let findings = vec![
            review(
                "finding:bot-comment",
                Some(BOT_ID),
                Some("Bot"),
                "coderabbitai",
                "NONE",
                "COMMENTED",
                Some(HEAD_SHA),
            ),
            review(
                "finding:resolver-shaped",
                Some(BOT_ID),
                Some("User"),
                "coderabbitai[bot]",
                "NONE",
                "APPROVED",
                Some(HEAD_SHA),
            ),
        ];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(
            *role_of(&result, "finding:bot-comment"),
            EvidenceRole::AutomatedBot
        );
        assert_eq!(
            *role_of(&result, "finding:resolver-shaped"),
            EvidenceRole::AutomatedBot
        );
        let resolver_classification = result
            .classifications
            .iter()
            .find(|classification| classification.subject_id == "finding:resolver-shaped")
            .unwrap();
        assert_eq!(
            resolver_classification.basis,
            ClassificationBasis::ProviderBotIdPrefix
        );
        // One actor, not two: neither occurrence of BOT_kgDOCCSy2w ever
        // reaches independent_human_approvals, despite one being
        // User-typed and APPROVED at head.
        assert!(result.independent_human_approvals.is_empty());
    }

    #[test]
    fn id_equality_rule_fires_for_a_sibling_occurrence_with_no_own_bot_signal() {
        // A synthetic id with no BOT_ prefix: one occurrence establishes bot
        // attestation via typename (arm 3 rule 1); a second finding by the
        // same id, User-typed and with no prefix of its own, must still
        // classify automated_bot via id equality (arm 3 rule 3) rather than
        // independent_human_candidate — proving attestation is sticky and a
        // later User typename never overrides it.
        let observation = observation();
        let findings = vec![
            review(
                "finding:establishes-attestation",
                Some("actor:mystery"),
                Some("Bot"),
                "mystery",
                "NONE",
                "COMMENTED",
                Some(HEAD_SHA),
            ),
            review(
                "finding:sibling-approval",
                Some("actor:mystery"),
                Some("User"),
                "mystery-renamed",
                "NONE",
                "APPROVED",
                Some(HEAD_SHA),
            ),
        ];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert_eq!(
            *role_of(&result, "finding:sibling-approval"),
            EvidenceRole::AutomatedBot
        );
        let classification = result
            .classifications
            .iter()
            .find(|classification| classification.subject_id == "finding:sibling-approval")
            .unwrap();
        assert_eq!(
            classification.basis,
            ClassificationBasis::ProviderBotIdEquality
        );
        assert!(result.independent_human_approvals.is_empty());
    }

    #[test]
    fn ci_check_subjects_classify_ci_check_and_never_enter_approvals() {
        let observation = observation();
        let checks = vec![check(
            "check:1",
            Some(Actor {
                id: BOT_ID.to_owned(),
                login: "coderabbitai".to_owned(),
                typename: "Bot".to_owned(),
            }),
        )];
        let result = evaluate_independence(&observation, &[], &checks, true);
        assert_eq!(result.classifications.len(), 1);
        assert_eq!(result.classifications[0].subject_id, "check:1");
        assert_eq!(
            result.classifications[0].evidence_role,
            EvidenceRole::CiCheck
        );
        assert_eq!(
            result.classifications[0].basis,
            ClassificationBasis::CheckObservation
        );
        assert!(result.independent_human_approvals.is_empty());
    }

    #[test]
    fn unresolved_and_resolved_actionable_findings_split_correctly() {
        let observation = observation();
        let findings = vec![
            thread_comment(
                "finding:resolved",
                Some("actor:bot-1"),
                Some("Bot"),
                "reviewbot",
                "thread-1",
                true,
                true,
            ),
            thread_comment(
                "finding:unresolved",
                Some("actor:bot-1"),
                Some("Bot"),
                "reviewbot",
                "thread-2",
                false,
                true,
            ),
            review(
                "finding:summary",
                Some("actor:bot-1"),
                Some("Bot"),
                "reviewbot",
                "NONE",
                "COMMENTED",
                Some(HEAD_SHA),
            ),
        ];
        let result = evaluate_independence(&observation, &findings, &[], false);
        assert_eq!(
            result.resolved_actionable_finding_ids,
            vec!["finding:resolved".to_owned()]
        );
        assert_eq!(
            result.unresolved_actionable_finding_ids,
            vec!["finding:unresolved".to_owned()]
        );
    }

    #[test]
    fn require_independent_review_false_is_satisfied_without_approvals() {
        let observation = observation();
        let result = evaluate_independence(&observation, &[], &[], false);
        assert!(result.policy.satisfied);
        assert!(result.policy.satisfying_finding_ids.is_empty());
    }

    #[test]
    fn require_independent_review_true_without_approvals_is_unsatisfied() {
        let observation = observation();
        let result = evaluate_independence(&observation, &[], &[], true);
        assert!(!result.policy.satisfied);
        assert!(result.policy.satisfying_finding_ids.is_empty());
    }

    #[test]
    fn always_carries_the_verbatim_independent_minds_finding_and_never_proves_independence() {
        let observation = observation();
        let findings = vec![review(
            "finding:1",
            Some("actor:outside"),
            Some("User"),
            "carol",
            "NONE",
            "APPROVED",
            Some(HEAD_SHA),
        )];
        let result = evaluate_independence(&observation, &findings, &[], true);
        assert!(!result.independent_human_approvals.is_empty());
        assert!(
            !result.independence_proven,
            "no configuration may ever prove independence"
        );
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].code, "independent_minds_not_observable");
        assert_eq!(
            result.findings[0].detail,
            "different actor ids do not prove independent minds or undeclared information isolation"
        );
    }

    /// Exhaustiveness (acceptance criterion 6): only `IndependentHumanCandidate`
    /// can ever satisfy the policy. Every role classifies an otherwise
    /// identical `APPROVED`-at-head finding, and this proves the real
    /// `evaluate_independence` counting logic — not a hand-written summary
    /// of it — treats exactly one role as satisfying.
    #[test]
    fn only_independent_human_candidate_role_can_satisfy_the_policy() {
        let observation = observation();
        let findings = vec![
            // SelfReview
            review(
                "finding:self-review",
                Some(PR_AUTHOR_ID),
                Some("User"),
                "rizumita",
                "MEMBER",
                "APPROVED",
                Some(HEAD_SHA),
            ),
            // AutomatedBot
            review(
                "finding:automated-bot",
                Some("actor:reviewbot"),
                Some("Bot"),
                "reviewbot",
                "NONE",
                "APPROVED",
                Some(HEAD_SHA),
            ),
            // Unattributed
            review(
                "finding:unattributed",
                None,
                None,
                "deleted-account",
                "NONE",
                "APPROVED",
                Some(HEAD_SHA),
            ),
            // IndependentHumanCandidate — the only satisfying shape.
            review(
                "finding:candidate",
                Some("actor:outside"),
                Some("User"),
                "carol",
                "NONE",
                "APPROVED",
                Some(HEAD_SHA),
            ),
        ];
        // CiCheck: checks have no review_state at all, so they are
        // structurally unable to appear in independent_human_approvals —
        // proven by construction below (only findings are scanned there).
        let checks = vec![check("check:1", None)];

        let result = evaluate_independence(&observation, &findings, &checks, true);

        assert_eq!(
            *role_of(&result, "finding:self-review"),
            EvidenceRole::SelfReview
        );
        assert_eq!(
            *role_of(&result, "finding:automated-bot"),
            EvidenceRole::AutomatedBot
        );
        assert_eq!(
            *role_of(&result, "finding:unattributed"),
            EvidenceRole::Unattributed
        );
        assert_eq!(*role_of(&result, "check:1"), EvidenceRole::CiCheck);
        assert_eq!(
            *role_of(&result, "finding:candidate"),
            EvidenceRole::IndependentHumanCandidate
        );

        assert_eq!(
            result.independent_human_approvals,
            vec!["finding:candidate".to_owned()],
            "only the IndependentHumanCandidate subject may satisfy the policy"
        );
        assert!(result.policy.satisfied);
    }

    /// Not a second behavioural proof — the `assert_eq!` below is tautological
    /// by construction (it compares a hand-written `match` against itself)
    /// and cannot fail for any implementation of `evaluate_independence`;
    /// the behavioural proof is `only_independent_human_candidate_role_can_satisfy_the_policy`
    /// above, which drives the real function. What is load-bearing here is
    /// that the `match` has no wildcard arm, so adding a sixth `EvidenceRole`
    /// variant fails **to compile** in this file rather than silently
    /// leaving the new variant unclassified. This test exists to make that
    /// compile-time fence discoverable by name, not to check behaviour.
    #[test]
    fn evidence_role_variants_are_exhaustively_covered_by_the_satisfaction_rule() {
        for role in [
            EvidenceRole::SelfReview,
            EvidenceRole::AutomatedBot,
            EvidenceRole::CiCheck,
            EvidenceRole::IndependentHumanCandidate,
            EvidenceRole::Unattributed,
        ] {
            let can_satisfy = match role {
                EvidenceRole::IndependentHumanCandidate => true,
                EvidenceRole::SelfReview
                | EvidenceRole::AutomatedBot
                | EvidenceRole::CiCheck
                | EvidenceRole::Unattributed => false,
            };
            assert_eq!(can_satisfy, role == EvidenceRole::IndependentHumanCandidate);
        }
    }

    #[test]
    fn implementation_actor_ids_reads_the_observation_field_without_recomputing() {
        let observation = observation_with_actors(
            &[PR_AUTHOR_ID, "actor:committer"],
            &["rizumita", "committer-login"],
        );
        let ids = implementation_actor_ids(&observation);
        assert_eq!(
            ids,
            BTreeSet::from([PR_AUTHOR_ID.to_owned(), "actor:committer".to_owned()])
        );
    }
}
