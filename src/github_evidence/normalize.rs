//! Manifest + captured bytes → `pr_observation` / `check_evidence` /
//! `review_finding` / `memory.source_record.v0`. The determinism surface
//! (design §5): every computation here reads only the manifest and the
//! artifact bytes it names — no clock, no environment, no network, no store.
//! Replaying the same retained files reproduces the same records
//! byte-for-byte.
//!
//! Provider JSON is parsed tolerantly (an allowlist of fields per artifact
//! category; unknown provider fields are the standing `provider_fields_unmapped`
//! loss, never a refusal) — the same ownership rule
//! `github_issue_snapshot.rs` documents: CaseGraphen's own wrapper
//! (`CaptureManifest`) is strict, the provider mirror is not. Two provider
//! shapes appear in the pilot corpus and both must parse: `gh --json` object
//! shape (`pr`, `files`, `issue` categories) and `gh api graphql` envelopes
//! (`reviews`, `review_threads`, `commits`, `checks` categories) — the four
//! that need the Actor `__typename`/`id` discriminators the `gh --json`
//! projections do not emit (design §3.1).
//!
//! Refusal vs `unattributed` (design §6): the artifacts that feed the
//! implementation actor-id set — the `pr` artifact's author and the
//! `commits` artifact's user objects — must carry GitHub node ids; missing
//! one there is a hard refusal (integrity class). A finding author (review,
//! thread comment) missing its discriminator/id instead normalizes with
//! those fields absent; independence classification (T3) reads that absence
//! as `Unattributed`. A finding author without an id also never
//! duplicate-collapses with anything, because the collapse key is
//! `(author.id, body_content_hash, path)` and collapsing requires actor id
//! equality.

use super::model::*;
use crate::memory::{
    self, AuthorityOrigin, MemorySourceKind, MemoryValidationFinding, Sensitivity, SourceRecord,
    MEMORY_SOURCE_RECORD_SCHEMA,
};
use crate::native_hash::{content_matches_sha256, sha256_hex};
use crate::path_confinement::path_confined;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

/// One manifest → source records + the normalized review snapshot. Store-free
/// (this module must not import `native_store`) and read-only: nothing here
/// ever writes to `capture_dir`.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedCapture {
    pub source_records: Vec<SourceRecord>,
    pub pr_observation: PrObservation,
    pub check_evidence: Vec<CheckEvidence>,
    pub review_findings: Vec<ReviewFinding>,
    /// Successful-but-obstructed observations (CLAUDE.md: domain findings are
    /// successful results carrying obstructions) — currently only
    /// `cross_repository_reference` (design §7): a captured finding whose URL
    /// names a different repository than the manifest declares is excluded
    /// from `review_findings` and recorded here instead of silently included
    /// or silently dropped.
    pub domain_findings: Vec<MemoryValidationFinding>,
    /// The URLs of every review finding excluded for the
    /// `cross_repository_reference` reason above, sorted — the structured
    /// form of the same fact `domain_findings` states in prose, kept so
    /// `projection.rs` can cite the excluded URLs in
    /// `review_projection.v0.losses` (`omitted_refs`) without parsing a
    /// `MemoryValidationFinding.detail` string. Both channels carry the same
    /// fact: `result.domain_findings` on the command envelope, and
    /// `losses` on the projection record itself, since the record is a
    /// standalone artifact a downstream reviewer may read without the
    /// envelope around it (design §7/§8).
    pub cross_repository_excluded: Vec<String>,
    /// Provider-reported `totalCount` versus the node count this capture
    /// actually received, retained so T4's `threads_truncated`/
    /// `files_truncated` losses (design §8) can be derived from this output
    /// alone — T4 must not re-parse the raw artifacts itself, or provider
    /// parsing would exist in two places (CLAUDE.md: a decision rule has
    /// exactly one implementation). Covers every category whose captured
    /// GraphQL connection reports a `totalCount` alongside its `nodes`
    /// (`reviews`, `review_threads`, and each thread's `comments`); the `pr`
    /// artifact's `files` array carries no such count in the `gh --json`
    /// shape this adapter reads, so file-level truncation is not detectable
    /// from this capture family at all — not an omission here, a limit of
    /// what `gh pr view --json files` reports.
    pub capture_totals: CaptureTotals,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaptureTotals {
    pub reviews_reported_total: u64,
    pub reviews_received: u64,
    pub review_threads_reported_total: u64,
    pub review_threads_received: u64,
    pub thread_comment_totals: Vec<ThreadCommentTotals>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThreadCommentTotals {
    pub thread_id: String,
    pub reported_total: u64,
    pub received: u64,
}

/// Normalizes one capture manifest against the artifact bytes under
/// `capture_dir`. Hard refusals (integrity class — CLAUDE.md: only stale
/// revisions and integrity mismatches are tool failures) short-circuit on the
/// first violation found; a capture that fails integrity is never partially
/// normalized into a chimera.
pub fn normalize(
    manifest: &CaptureManifest,
    capture_dir: &Path,
) -> Result<NormalizedCapture, MemoryValidationFinding> {
    validate_repository_format(&manifest.repository)?;
    {
        let mut timestamp_findings = Vec::new();
        memory::validate_timestamp(
            &manifest.captured_at,
            "$.captured_at",
            &mut timestamp_findings,
        );
        if let Some(first) = timestamp_findings.into_iter().next() {
            return Err(first);
        }
    }
    validate_category_shape(manifest)?;

    let canonical_capture_dir = fs::canonicalize(capture_dir).map_err(|source| {
        refusal(
            "capture_dir_unreadable",
            "$.capture_dir",
            &format!(
                "capture directory {} could not be canonicalized: {source}",
                capture_dir.display()
            ),
        )
    })?;

    // Phase 1: verify integrity and read every entry's bytes before any
    // category-specific parsing runs. One `memory.source_record.v0` per
    // manifest entry (design §3.2/T2 plan), even when two entries — `pr` and
    // `files` in the pilot manifest — share the same `artifact_path`.
    let entries_read = manifest
        .entries
        .iter()
        .map(|entry| read_entry(entry, &canonical_capture_dir, &manifest.captured_at))
        .collect::<Result<Vec<_>, _>>()?;

    let pr_read = one_of(&entries_read, CaptureCategory::Pr)?;
    let pr_mirror: GhPrMirror = parse_tolerant(&pr_read.bytes, "pr")?;
    if pr_mirror.number != manifest.pr_number {
        return Err(refusal(
            "pr_number_mismatch",
            "$.entries[category=pr]",
            "pr artifact number does not match manifest pr_number",
        ));
    }
    let pr_repository = extract_repo_from_url(&pr_mirror.url).ok_or_else(|| {
        refusal(
            "pr_url_unparseable",
            "$.entries[category=pr].url",
            "pr url is not a recognizable GitHub pull request url",
        )
    })?;
    if pr_repository != manifest.repository {
        return Err(refusal(
            "repository_mismatch",
            "$.entries[category=pr].url",
            "pr artifact repository does not match manifest repository",
        ));
    }
    let pr_author_id = pr_mirror.author.id.clone().ok_or_else(|| {
        refusal(
            "actor_set_source_missing_id",
            "$.entries[category=pr].author.id",
            "pr author is missing its GitHub node id; the implementation actor set cannot be built",
        )
    })?;

    let files_read = one_of(&entries_read, CaptureCategory::Files)?;
    let files_mirror: GhFilesMirror = parse_tolerant(&files_read.bytes, "files")?;
    let mut changed_files: Vec<ChangedFile> = files_mirror
        .files
        .into_iter()
        .map(|file| ChangedFile {
            path: file.path,
            additions: file.additions,
            deletions: file.deletions,
            change_type: file.change_type,
        })
        .collect();
    changed_files.sort_by(|left, right| left.path.cmp(&right.path));

    let commits_read = one_of(&entries_read, CaptureCategory::Commits)?;
    let commits_envelope: GraphQlCommitsEnvelope = parse_tolerant(&commits_read.bytes, "commits")?;
    let mut actor_ids: BTreeSet<String> = BTreeSet::new();
    let mut actor_logins: BTreeSet<String> = BTreeSet::new();
    actor_ids.insert(pr_author_id.clone());
    actor_logins.insert(pr_mirror.author.login.clone());
    for node in commits_envelope.data.repository.pull_request.commits.nodes {
        let (author_id, author_login) = require_commit_actor(node.commit.author.user, "author")?;
        let (committer_id, committer_login) =
            require_commit_actor(node.commit.committer.user, "committer")?;
        actor_ids.insert(author_id);
        actor_logins.insert(author_login);
        actor_ids.insert(committer_id);
        actor_logins.insert(committer_login);
    }

    let checks_read = one_of(&entries_read, CaptureCategory::Checks)?;
    let checks_envelope: GraphQlChecksEnvelope = parse_tolerant(&checks_read.bytes, "checks")?;
    let checks_pr = checks_envelope.data.repository.pull_request;
    if checks_pr.head_ref_oid != pr_mirror.head_ref_oid {
        return Err(refusal(
            "intra_capture_head_mismatch",
            "$.entries[category=checks].headRefOid",
            "checks artifact headRefOid disagrees with the pr artifact head",
        ));
    }
    let checks_commit = checks_pr.commits.nodes.first().ok_or_else(|| {
        refusal(
            "checks_missing_head_commit",
            "$.entries[category=checks].commits",
            "checks artifact carries no commit data for the observed head",
        )
    })?;
    if checks_commit.commit.oid != pr_mirror.head_ref_oid {
        return Err(refusal(
            "intra_capture_head_mismatch",
            "$.entries[category=checks].commits[0].commit.oid",
            "checks artifact commit oid disagrees with the observed head",
        ));
    }
    let mut check_evidence = Vec::new();
    if let Some(rollup) = &checks_commit.commit.status_check_rollup {
        for context in &rollup.contexts.nodes {
            check_evidence.push(build_check_evidence(
                context,
                &pr_mirror.head_ref_oid,
                &checks_read.source_record_id,
            )?);
        }
    }
    check_evidence.sort_by(|left, right| {
        (left.kind, &left.name, &left.completed_at, &left.details_url).cmp(&(
            right.kind,
            &right.name,
            &right.completed_at,
            &right.details_url,
        ))
    });

    let reviews_read = one_of(&entries_read, CaptureCategory::Reviews)?;
    let reviews_envelope: GraphQlReviewsEnvelope = parse_tolerant(&reviews_read.bytes, "reviews")?;
    let reviews_pr = reviews_envelope.data.repository.pull_request;
    if reviews_pr.number != manifest.pr_number {
        return Err(refusal(
            "intra_capture_pr_number_mismatch",
            "$.entries[category=reviews].number",
            "reviews artifact pr number disagrees with the manifest",
        ));
    }
    let reviews_reported_total = reviews_pr.reviews.total_count;
    let reviews_received = reviews_pr.reviews.nodes.len() as u64;
    let mut review_findings: Vec<ReviewFinding> = reviews_pr
        .reviews
        .nodes
        .into_iter()
        .map(|node| review_summary_finding(node, &reviews_read.source_record_id))
        .collect();

    let threads_read = one_of(&entries_read, CaptureCategory::ReviewThreads)?;
    let threads_envelope: GraphQlThreadsEnvelope =
        parse_tolerant(&threads_read.bytes, "review_threads")?;
    let threads_pr = threads_envelope.data.repository.pull_request;
    if threads_pr.number != manifest.pr_number
        || threads_pr.base_ref_oid != pr_mirror.base_ref_oid
        || threads_pr.head_ref_oid != pr_mirror.head_ref_oid
    {
        return Err(refusal(
            "intra_capture_head_mismatch",
            "$.entries[category=review_threads]",
            "review_threads artifact head/base/number disagrees with the pr artifact \
             (the capture straddled a push)",
        ));
    }
    let review_threads_reported_total = threads_pr.review_threads.total_count;
    let review_threads_received = threads_pr.review_threads.nodes.len() as u64;
    let mut thread_comment_totals = Vec::new();
    for thread in threads_pr.review_threads.nodes {
        let resolved_by = match thread.resolved_by {
            Some(actor) => match actor.id {
                Some(id) => Some(ResolvedBy {
                    id,
                    login: actor.login,
                }),
                None => None,
            },
            None => None,
        };
        let thread_state = ReviewThreadState {
            thread_id: thread.id.clone(),
            resolved: thread.is_resolved,
            outdated: thread.is_outdated,
            resolved_by,
            comment_count: thread.comments.total_count,
        };
        thread_comment_totals.push(ThreadCommentTotals {
            thread_id: thread.id,
            reported_total: thread.comments.total_count,
            received: thread.comments.nodes.len() as u64,
        });
        for (index, comment) in thread.comments.nodes.into_iter().enumerate() {
            review_findings.push(thread_comment_finding(
                comment,
                index == 0,
                thread_state.clone(),
                thread.path.clone(),
                &threads_read.source_record_id,
            ));
        }
    }
    thread_comment_totals.sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    let capture_totals = CaptureTotals {
        reviews_reported_total,
        reviews_received,
        review_threads_reported_total,
        review_threads_received,
        thread_comment_totals,
    };

    // Cross-repository references (design §7): excluded and declared, never
    // silently included and never a refusal — a thread or review can
    // legitimately quote another repository's URL. `cross_repository_excluded`
    // is the structured record of the same fact `domain_findings` narrates,
    // so a consumer of the projection record alone (not just the command
    // envelope) can still see what was dropped and why (design §8).
    let mut domain_findings = Vec::new();
    let mut cross_repository_excluded = Vec::new();
    let review_findings: Vec<ReviewFinding> = review_findings
        .into_iter()
        .filter(|finding| match extract_repo_from_url(&finding.url) {
            Some(repository) if repository != manifest.repository => {
                domain_findings.push(MemoryValidationFinding {
                    code: "cross_repository_reference".to_owned(),
                    location: "$.review_findings".to_owned(),
                    detail: format!(
                        "excluded finding at {} (repository {repository} does not match \
                         manifest repository {})",
                        finding.url, manifest.repository
                    ),
                });
                cross_repository_excluded.push(finding.url.clone());
                false
            }
            _ => true,
        })
        .collect();
    cross_repository_excluded.sort();
    let review_findings = collapse_duplicate_findings(review_findings);

    let mut issues = Vec::with_capacity(manifest.issue_numbers.len());
    for number in &manifest.issue_numbers {
        let issue_read = entries_read
            .iter()
            .find(|entry| {
                entry.category == CaptureCategory::Issue && entry.issue_number == Some(*number)
            })
            .expect("validate_category_shape already proved every issue_number has an entry");
        let mirror: GhIssueMirror = parse_tolerant(&issue_read.bytes, "issue")?;
        if mirror.number != *number {
            return Err(refusal(
                "issue_number_mismatch",
                "$.entries[category=issue]",
                "issue artifact number does not match its manifest issue_number",
            ));
        }
        let mut closed_by_pr_numbers: BTreeSet<u64> = mirror
            .closed_by_pull_requests_references
            .into_iter()
            .map(|reference| reference.number)
            .collect();
        issues.push(PrObservationIssue {
            number: mirror.number,
            title: mirror.title,
            state: mirror.state,
            state_reason: mirror.state_reason,
            url: mirror.url,
            created_at: mirror.created_at,
            closed_at: mirror.closed_at,
            body_content_hash: content_hash_of(&mirror.body),
            closed_by_pr_numbers: closed_by_pr_numbers.iter().copied().collect(),
        });
        closed_by_pr_numbers.clear();
    }
    issues.sort_by_key(|issue| issue.number);

    let mut source_record_ids: Vec<String> = entries_read
        .iter()
        .map(|entry| entry.source_record_id.clone())
        .collect();
    source_record_ids.sort();

    let base = RefSha {
        git_ref: pr_mirror.base_ref_name.clone(),
        sha: pr_mirror.base_ref_oid.clone(),
    };
    let head = RefSha {
        git_ref: pr_mirror.head_ref_name.clone(),
        sha: pr_mirror.head_ref_oid.clone(),
    };
    let observation_id = format!(
        "github-observation:{}#{}@{}",
        manifest.repository, manifest.pr_number, pr_mirror.head_ref_oid
    );

    let mut observation = PrObservation {
        schema: GITHUB_PR_OBSERVATION_SCHEMA.to_owned(),
        observation_id,
        repository: manifest.repository.clone(),
        issues,
        pr: PrObservationPr {
            number: pr_mirror.number,
            title: pr_mirror.title.clone(),
            url: pr_mirror.url.clone(),
            state: pr_mirror.state.clone(),
            author: PrAuthor {
                id: pr_author_id,
                login: pr_mirror.author.login.clone(),
            },
            created_at: pr_mirror.created_at.clone(),
            body_content_hash: content_hash_of(&pr_mirror.body),
        },
        base,
        head,
        liveness: Liveness {
            state: pr_mirror.state.clone(),
            mergeable: pr_mirror.mergeable,
            merge_state_status: pr_mirror.merge_state_status.clone(),
            merged_at: pr_mirror.merged_at.clone(),
            closed_at: pr_mirror.closed_at.clone(),
            merge_commit_sha: None,
        },
        changed_files,
        implementation_actors: ImplementationActors {
            actor_ids: actor_ids.into_iter().collect(),
            logins: actor_logins.into_iter().collect(),
            derivation: "pr_author_and_commit_authors_and_committers".to_owned(),
        },
        source_record_ids,
        captured_at: manifest.captured_at.clone(),
        provider_fields_unmapped: true,
        normalized_content_hash: String::new(),
    };
    observation.normalized_content_hash = observation_content_hash(&observation);

    Ok(NormalizedCapture {
        source_records: entries_read
            .into_iter()
            .map(|entry| entry.source_record)
            .collect(),
        pr_observation: observation,
        check_evidence,
        review_findings,
        domain_findings,
        cross_repository_excluded,
        capture_totals,
    })
}

// ---------------------------------------------------------------------
// Manifest-shape validation (no artifact bytes needed)
// ---------------------------------------------------------------------

fn validate_repository_format(repository: &str) -> Result<(), MemoryValidationFinding> {
    let mut parts = repository.splitn(3, '/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty() => Ok(()),
        _ => Err(refusal(
            "invalid_repository_format",
            "$.repository",
            "repository must be exactly \"owner/name\"",
        )),
    }
}

/// The six single-artifact categories must appear exactly once; `issue`
/// entries must correspond one-to-one with `manifest.issue_numbers`, and
/// `issue_number` must be present exactly on `issue` entries.
fn validate_category_shape(manifest: &CaptureManifest) -> Result<(), MemoryValidationFinding> {
    for category in [
        CaptureCategory::Pr,
        CaptureCategory::Files,
        CaptureCategory::Reviews,
        CaptureCategory::ReviewThreads,
        CaptureCategory::Commits,
        CaptureCategory::Checks,
    ] {
        let count = manifest
            .entries
            .iter()
            .filter(|entry| entry.category == category)
            .count();
        if count != 1 {
            return Err(refusal(
                "invalid_category_count",
                "$.entries",
                &format!("manifest must declare exactly one {category:?} entry, found {count}"),
            ));
        }
    }
    let mut seen_issue_numbers = BTreeSet::new();
    for entry in &manifest.entries {
        match entry.category {
            CaptureCategory::Issue => {
                let number = entry.issue_number.ok_or_else(|| {
                    refusal(
                        "missing_issue_number",
                        "$.entries[category=issue].issue_number",
                        "an issue entry must declare issue_number",
                    )
                })?;
                if !manifest.issue_numbers.contains(&number) {
                    return Err(refusal(
                        "undeclared_issue_number",
                        "$.entries[category=issue].issue_number",
                        "issue entry issue_number is not listed in manifest.issue_numbers",
                    ));
                }
                if !seen_issue_numbers.insert(number) {
                    return Err(refusal(
                        "duplicate_issue_entry",
                        "$.entries[category=issue].issue_number",
                        "manifest declares more than one entry for the same issue_number",
                    ));
                }
            }
            _ => {
                if entry.issue_number.is_some() {
                    return Err(refusal(
                        "unexpected_issue_number",
                        "$.entries[].issue_number",
                        "issue_number is only meaningful on an issue entry",
                    ));
                }
            }
        }
    }
    for number in &manifest.issue_numbers {
        if !seen_issue_numbers.contains(number) {
            return Err(refusal(
                "missing_issue_entry",
                "$.entries",
                &format!(
                    "manifest.issue_numbers declares issue {number} without a matching issue entry"
                ),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Confined artifact resolution + one source record per entry
// ---------------------------------------------------------------------

struct EntryRead {
    category: CaptureCategory,
    issue_number: Option<u64>,
    bytes: Vec<u8>,
    source_record_id: String,
    source_record: SourceRecord,
}

fn read_entry(
    entry: &CaptureEntry,
    canonical_capture_dir: &Path,
    captured_at: &str,
) -> Result<EntryRead, MemoryValidationFinding> {
    let canonical_artifact =
        resolve_confined_artifact(canonical_capture_dir, &entry.artifact_path)?;
    let bytes = fs::read(&canonical_artifact).map_err(|source| {
        refusal(
            "artifact_unreadable",
            "$.entries[].artifact_path",
            &format!("{}: {source}", entry.artifact_path),
        )
    })?;
    let expected_hex = entry
        .content_hash
        .strip_prefix("sha256:")
        .unwrap_or(entry.content_hash.as_str());
    if !content_matches_sha256(&bytes, expected_hex) {
        return Err(refusal(
            "content_hash_mismatch",
            "$.entries[].content_hash",
            &format!(
                "{} does not match its declared content_hash",
                entry.artifact_path
            ),
        ));
    }
    let source_record_id = format!(
        "github-source:{}:sha256-{expected_hex}",
        category_str(entry.category)
    );
    let source_record = SourceRecord {
        schema: MEMORY_SOURCE_RECORD_SCHEMA.to_owned(),
        source_record_id: source_record_id.clone(),
        source_kind: MemorySourceKind::ToolOutput,
        content_hash: entry.content_hash.clone(),
        captured_at: captured_at.to_owned(),
        origin_actor_id: "actor:github-capture".to_owned(),
        source_boundary_id: "source_boundary:github-provider".to_owned(),
        authority_origin: AuthorityOrigin::Tool,
        sensitivity: Sensitivity::Internal,
        artifact_ref: entry.artifact_path.clone(),
    };
    if let Some(first) = memory::validate_memory_source_record(&source_record, &bytes)
        .into_iter()
        .next()
    {
        return Err(first);
    }
    Ok(EntryRead {
        category: entry.category,
        issue_number: entry.issue_number,
        bytes,
        source_record_id,
        source_record,
    })
}

fn one_of(
    entries: &[EntryRead],
    category: CaptureCategory,
) -> Result<&EntryRead, MemoryValidationFinding> {
    let mut matches = entries.iter().filter(|entry| entry.category == category);
    let first = matches.next().ok_or_else(|| {
        refusal(
            "missing_required_category",
            "$.entries",
            &format!("manifest is missing a required {category:?} entry"),
        )
    })?;
    if matches.next().is_some() {
        return Err(refusal(
            "duplicate_category_entry",
            "$.entries",
            &format!("manifest declares more than one {category:?} entry"),
        ));
    }
    Ok(first)
}

/// Confines a manifest-relative `artifact_path` to `canonical_capture_dir`.
/// The same three-stage resolution `native_cli/ops/mutations.rs::prepare_claim`
/// documents (lexical rejection, then canonicalize-joined-onto-root, then
/// containment) — stage 3 is `crate::path_confinement::path_confined`, the
/// one shared implementation of the predicate (CLAUDE.md: a decision rule
/// has exactly one implementation).
fn resolve_confined_artifact(
    canonical_capture_dir: &Path,
    relative_path: &str,
) -> Result<PathBuf, MemoryValidationFinding> {
    let candidate = Path::new(relative_path);
    if candidate.is_absolute()
        || candidate
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(refusal(
            "artifact_path_escape",
            "$.entries[].artifact_path",
            relative_path,
        ));
    }
    fs::canonicalize(canonical_capture_dir.join(candidate))
        .ok()
        .filter(|canonical| path_confined(canonical, canonical_capture_dir))
        .ok_or_else(|| {
            refusal(
                "artifact_path_escape",
                "$.entries[].artifact_path",
                relative_path,
            )
        })
}

fn parse_tolerant<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    category: &str,
) -> Result<T, MemoryValidationFinding> {
    serde_json::from_slice(bytes).map_err(|error| {
        refusal(
            "malformed_provider_capture",
            &format!("$.entries[category={category}]"),
            &format!("{category} artifact does not match the expected provider shape: {error}"),
        )
    })
}

/// Reuses the category enum's own `#[serde(rename_all = "snake_case")]`
/// mapping for the `source_record_id` prefix, so the id text can never drift
/// from the wire representation `CaptureCategory` already commits to.
fn category_str(category: CaptureCategory) -> String {
    serde_json::to_value(category)
        .expect("CaptureCategory serializes")
        .as_str()
        .expect("CaptureCategory serializes to a string")
        .to_owned()
}

fn refusal(code: &str, location: &str, detail: &str) -> MemoryValidationFinding {
    MemoryValidationFinding {
        code: code.to_owned(),
        location: location.to_owned(),
        detail: detail.to_owned(),
    }
}

fn content_hash_of(text: &str) -> String {
    format!("sha256:{}", sha256_hex(text.as_bytes()))
}

fn finding_id_for(url: &str) -> String {
    format!("finding:{}", sha256_hex(url.as_bytes()))
}

fn extract_repo_from_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.splitn(3, '/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

/// The `projection_content_hash` pattern (`memory::validation`):
/// `sha256:` of the record's own canonical serialization with its hash field
/// cleared first.
///
/// `pub(crate)`, not private: `refresh.rs`'s basis-integrity check
/// (design §6.1/§7) needs the exact same computation to recompute what a
/// caller-supplied previous observation's `normalized_content_hash` claims
/// to be. It used to keep its own byte-for-byte copy of this function
/// (CLAUDE.md forbids a decision rule having two implementations); this is
/// the single implementation both call.
pub(crate) fn observation_content_hash(observation: &PrObservation) -> String {
    let mut cleared = observation.clone();
    cleared.normalized_content_hash.clear();
    let bytes = serde_json::to_vec(&cleared).expect("typed pr_observation serializes");
    format!("sha256:{}", sha256_hex(&bytes))
}

fn require_commit_actor(
    user: Option<GraphQlUserActorMirror>,
    role: &str,
) -> Result<(String, String), MemoryValidationFinding> {
    let user = user.ok_or_else(|| {
        refusal(
            "actor_set_source_missing_id",
            &format!("$.entries[category=commits].commit.{role}"),
            &format!("commit {role} is missing a GitHub user; the implementation actor set cannot be built"),
        )
    })?;
    let id = user.id.ok_or_else(|| {
        refusal(
            "actor_set_source_missing_id",
            &format!("$.entries[category=commits].commit.{role}.user.id"),
            &format!("commit {role} user is missing its GitHub node id"),
        )
    })?;
    Ok((id, user.login))
}

// ---------------------------------------------------------------------
// Review findings: build, then collapse duplicates (design §3.4)
// ---------------------------------------------------------------------

fn review_summary_finding(node: GraphQlReviewNode, source_record_id: &str) -> ReviewFinding {
    let author = ReviewFindingAuthor {
        id: node.author.id,
        login: node.author.login,
        typename: node.author.typename,
        association: node.author_association,
    };
    let body_content_hash = content_hash_of(&node.body);
    ReviewFinding {
        schema: GITHUB_REVIEW_FINDING_SCHEMA.to_owned(),
        finding_id: finding_id_for(&node.url),
        kind: FindingKind::ReviewSummary,
        author,
        authored_at: node.submitted_at,
        last_edited_at: node.last_edited_at.clone(),
        edited: node.last_edited_at.is_some(),
        url: node.url,
        path: None,
        review_state: Some(node.state),
        commit_sha: node.commit.oid,
        body_content_hash,
        actionable: false,
        thread: None,
        duplicate_count: 1,
        source_record_id: source_record_id.to_owned(),
    }
}

fn thread_comment_finding(
    comment: GraphQlThreadCommentNode,
    is_opener: bool,
    thread_state: ReviewThreadState,
    path: Option<String>,
    source_record_id: &str,
) -> ReviewFinding {
    let author = ReviewFindingAuthor {
        id: comment.author.id,
        login: comment.author.login,
        typename: comment.author.typename,
        association: comment.author_association,
    };
    let body_content_hash = content_hash_of(&comment.body);
    ReviewFinding {
        schema: GITHUB_REVIEW_FINDING_SCHEMA.to_owned(),
        finding_id: finding_id_for(&comment.url),
        kind: FindingKind::ThreadComment,
        author,
        authored_at: comment.created_at,
        last_edited_at: comment.last_edited_at.clone(),
        edited: comment.last_edited_at.is_some(),
        url: comment.url,
        path,
        review_state: None,
        commit_sha: None,
        body_content_hash,
        actionable: is_opener,
        thread: Some(thread_state),
        duplicate_count: 1,
        source_record_id: source_record_id.to_owned(),
    }
}

/// Normalization may compress only what no decision rule distinguishes.
/// The collapse key must therefore include every field that a decision
/// rule or acceptance criterion downstream (T3's `evaluate_independence`,
/// T4's projection tier rule) actually reads to decide something — not just
/// enough fields to make same-author-same-text duplicates *look* alike. A
/// key that omits a decision-relevant field can silently launder two
/// distinguishable observations into one: e.g. `(author.id,
/// body_content_hash, path)` alone would merge an empty-bodied `APPROVED`
/// review bound to an old commit with an empty-bodied `COMMENTED` review at
/// head from the same outside reviewer — and empty bodies are the common
/// case, not an edge case (18 of the real PR-101 corpus's 20 reviews have
/// one) — silently gaining or losing the one signal
/// `evaluate_independence`'s `commit_sha == head.sha` check exists to gate.
/// It would equally merge two identical bot nitpicks sitting in two
/// *different* threads, discarding the fact that they are two separate
/// unresolved-or-resolved obligations, not one obligation observed twice.
///
/// Today that means: `author.id` (actor identity, not login — a rename must
/// not split or merge duplicates, design §3.4/§6), `body_content_hash`,
/// `path`, `review_state` and `commit_sha` (both read by
/// `evaluate_independence`'s approval-at-head check), and — for
/// `thread_comment` findings — the thread's `thread_id` and `resolved` flag
/// (thread identity and resolution state are both read by the actionable-
/// finding split). A future field is decision-relevant, and therefore
/// belongs in this key, exactly when some rule or acceptance criterion
/// reads it to decide something; a field that is purely descriptive (e.g.
/// `thread.resolved_by`, `authorAssociation`) does not.
///
/// A finding author without an id (`author.id.is_none()`) never collapses
/// with anything regardless of the rest of the key, so its count is always
/// conservatively preserved as separate findings. The tie-break (lowest
/// `(authored_at, url)` survives) also happens to pick a thread's opening
/// comment as the representative when it duplicates a later reply in the
/// *same* thread, since the opener is always the earliest comment there —
/// this is why the real "duplicate bot findings" case (two identical Bot
/// comments in one thread, one of them the opener) still collapses
/// correctly under this key: same thread, same resolution state, same
/// everything else, differing only in position.
fn collapse_duplicate_findings(findings: Vec<ReviewFinding>) -> Vec<ReviewFinding> {
    let mut groups: std::collections::BTreeMap<CollapseKey, Vec<ReviewFinding>> =
        std::collections::BTreeMap::new();
    let mut unattributed = Vec::new();
    for finding in findings {
        match finding.author.id.clone() {
            Some(id) => {
                let key = collapse_key(&finding, id);
                groups.entry(key).or_default().push(finding);
            }
            None => unattributed.push(finding),
        }
    }
    let mut collapsed = Vec::new();
    for (_, mut group) in groups {
        group.sort_by(|left, right| {
            (&left.authored_at, &left.url).cmp(&(&right.authored_at, &right.url))
        });
        let duplicate_count = group.len() as u32;
        let mut representative = group.remove(0);
        representative.duplicate_count = duplicate_count;
        collapsed.push(representative);
    }
    collapsed.extend(unattributed);
    collapsed.sort_by(|left, right| {
        (&left.authored_at, &left.url).cmp(&(&right.authored_at, &right.url))
    });
    collapsed
}

type CollapseKey = (
    String,         // author.id
    String,         // body_content_hash
    Option<String>, // path
    Option<String>, // review_state
    Option<String>, // commit_sha
    Option<String>, // thread.thread_id (thread_comment findings only)
    Option<bool>,   // thread.resolved (thread_comment findings only)
);

fn collapse_key(finding: &ReviewFinding, author_id: String) -> CollapseKey {
    let (thread_id, thread_resolved) = match &finding.thread {
        Some(thread) => (Some(thread.thread_id.clone()), Some(thread.resolved)),
        None => (None, None),
    };
    (
        author_id,
        finding.body_content_hash.clone(),
        finding.path.clone(),
        finding.review_state.clone(),
        finding.commit_sha.clone(),
        thread_id,
        thread_resolved,
    )
}

// ---------------------------------------------------------------------
// Checks: heterogeneous CheckRun/StatusContext union, discriminated by the
// GraphQL `__typename` (design §3.3)
// ---------------------------------------------------------------------

fn build_check_evidence(
    context: &Value,
    head_sha: &str,
    source_record_id: &str,
) -> Result<CheckEvidence, MemoryValidationFinding> {
    let typename = context
        .get("__typename")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let str_field = |key: &str| context.get(key).and_then(Value::as_str).map(str::to_owned);
    match typename {
        "CheckRun" => {
            let name = str_field("name").ok_or_else(|| {
                refusal(
                    "malformed_check_context",
                    "$.entries[category=checks]",
                    "CheckRun context is missing name",
                )
            })?;
            let details_url = str_field("detailsUrl");
            let workflow_name = context
                .get("checkSuite")
                .and_then(|value| value.get("workflowRun"))
                .and_then(|value| value.get("workflow"))
                .and_then(|value| value.get("name"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let url_for_id = details_url.clone().unwrap_or_default();
            Ok(CheckEvidence {
                schema: GITHUB_CHECK_EVIDENCE_SCHEMA.to_owned(),
                check_id: format!(
                    "check:{head_sha}:check_run:{name}:{}",
                    sha256_hex(url_for_id.as_bytes())
                ),
                head_sha: head_sha.to_owned(),
                kind: CheckKind::CheckRun,
                name,
                workflow_name,
                status: str_field("status"),
                conclusion: str_field("conclusion"),
                state: None,
                creator: None,
                details_url,
                target_url: None,
                description: None,
                started_at: str_field("startedAt"),
                completed_at: str_field("completedAt"),
                created_at: None,
                evidence_role: "ci_check".to_owned(),
                source_record_id: source_record_id.to_owned(),
            })
        }
        "StatusContext" => {
            let name = str_field("context").ok_or_else(|| {
                refusal(
                    "malformed_check_context",
                    "$.entries[category=checks]",
                    "StatusContext context is missing context (name)",
                )
            })?;
            let target_url = str_field("targetUrl");
            let creator = context.get("creator").and_then(|creator| {
                let typename = creator
                    .get("__typename")
                    .and_then(Value::as_str)?
                    .to_owned();
                let login = creator.get("login").and_then(Value::as_str)?.to_owned();
                let id = creator.get("id").and_then(Value::as_str)?.to_owned();
                Some(Actor {
                    id,
                    login,
                    typename,
                })
            });
            let url_for_id = target_url.clone().unwrap_or_default();
            Ok(CheckEvidence {
                schema: GITHUB_CHECK_EVIDENCE_SCHEMA.to_owned(),
                check_id: format!(
                    "check:{head_sha}:status_context:{name}:{}",
                    sha256_hex(url_for_id.as_bytes())
                ),
                head_sha: head_sha.to_owned(),
                kind: CheckKind::StatusContext,
                name,
                workflow_name: None,
                status: None,
                conclusion: None,
                state: str_field("state"),
                creator,
                details_url: None,
                target_url,
                description: str_field("description"),
                started_at: None,
                completed_at: None,
                created_at: str_field("createdAt"),
                evidence_role: "ci_check".to_owned(),
                source_record_id: source_record_id.to_owned(),
            })
        }
        other => Err(refusal(
            "unsupported_check_context_type",
            "$.entries[category=checks]",
            &format!("unsupported check context __typename {other:?}"),
        )),
    }
}

// ---------------------------------------------------------------------
// Provider mirrors — tolerant, not `deny_unknown_fields`. Each struct reads
// only the allowlisted fields one manifest category needs; every other field
// present in the raw capture is the standing `provider_fields_unmapped` loss.
// ---------------------------------------------------------------------

/// `gh pr view --json …` shape (`pr` category; `pr-101.json` in the pilot).
#[derive(Debug, Deserialize)]
struct GhPrMirror {
    number: u64,
    title: String,
    url: String,
    state: String,
    author: GhActorMirror,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "baseRefOid")]
    base_ref_oid: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(default, rename = "mergedAt")]
    merged_at: Option<String>,
    #[serde(default, rename = "closedAt")]
    closed_at: Option<String>,
    mergeable: MergeableState,
    #[serde(rename = "mergeStateStatus")]
    merge_state_status: String,
    body: String,
}

/// `gh --json`'s actor shape (`{id, is_bot, login, name}`); only `id`/`login`
/// are read. `id` is optional here — the source of the "missing actor
/// attestation" refusal for `pr.author` (design §6).
#[derive(Debug, Deserialize)]
struct GhActorMirror {
    login: String,
    #[serde(default)]
    id: Option<String>,
}

/// `gh pr view --json files` shape (`files` category).
#[derive(Debug, Deserialize)]
struct GhFilesMirror {
    files: Vec<GhFileMirror>,
}

#[derive(Debug, Deserialize)]
struct GhFileMirror {
    path: String,
    additions: u64,
    deletions: u64,
    #[serde(rename = "changeType")]
    change_type: String,
}

/// `gh issue view --json …` shape (`issue` category).
#[derive(Debug, Deserialize)]
struct GhIssueMirror {
    number: u64,
    title: String,
    state: String,
    #[serde(default, rename = "stateReason")]
    state_reason: Option<String>,
    url: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(default, rename = "closedAt")]
    closed_at: Option<String>,
    body: String,
    #[serde(default, rename = "closedByPullRequestsReferences")]
    closed_by_pull_requests_references: Vec<GhClosedByPrMirror>,
}

#[derive(Debug, Deserialize)]
struct GhClosedByPrMirror {
    number: u64,
}

/// `gh api graphql` envelope shared by the `reviews`, `review_threads`,
/// `commits`, and `checks` categories:
/// `{"data":{"repository":{"pullRequest": T}}}`.
#[derive(Debug, Deserialize)]
struct GraphQlEnvelope<T> {
    data: GraphQlData<T>,
}

#[derive(Debug, Deserialize)]
struct GraphQlData<T> {
    repository: GraphQlRepository<T>,
}

#[derive(Debug, Deserialize)]
struct GraphQlRepository<T> {
    #[serde(rename = "pullRequest")]
    pull_request: T,
}

/// The GraphQL Actor shape `{__typename, login, id}`. `typename`/`id` are
/// each independently optional: the pilot's own unattested `reviews`
/// section of `pr-101.json` (a `gh --json` review, not GraphQL) carries only
/// `login`, and the design's `unattributed` classification exists precisely
/// for this shape appearing where GraphQL normally would carry both.
#[derive(Debug, Deserialize)]
struct GraphQlActorMirror {
    #[serde(default, rename = "__typename")]
    typename: Option<String>,
    login: String,
    #[serde(default)]
    id: Option<String>,
}

type GraphQlReviewsEnvelope = GraphQlEnvelope<GraphQlReviewsPr>;

#[derive(Debug, Deserialize)]
struct GraphQlReviewsPr {
    number: u64,
    reviews: GraphQlReviewsConnection,
}

#[derive(Debug, Deserialize)]
struct GraphQlReviewsConnection {
    #[serde(rename = "totalCount")]
    total_count: u64,
    nodes: Vec<GraphQlReviewNode>,
}

#[derive(Debug, Deserialize)]
struct GraphQlReviewNode {
    state: String,
    body: String,
    #[serde(rename = "submittedAt")]
    submitted_at: String,
    #[serde(default, rename = "lastEditedAt")]
    last_edited_at: Option<String>,
    url: String,
    #[serde(rename = "authorAssociation")]
    author_association: String,
    commit: GraphQlCommitOid,
    author: GraphQlActorMirror,
}

#[derive(Debug, Deserialize)]
struct GraphQlCommitOid {
    #[serde(default)]
    oid: Option<String>,
}

type GraphQlThreadsEnvelope = GraphQlEnvelope<GraphQlThreadsPr>;

#[derive(Debug, Deserialize)]
struct GraphQlThreadsPr {
    number: u64,
    #[serde(rename = "baseRefOid")]
    base_ref_oid: String,
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
    #[serde(rename = "reviewThreads")]
    review_threads: GraphQlThreadsConnection,
}

#[derive(Debug, Deserialize)]
struct GraphQlThreadsConnection {
    #[serde(rename = "totalCount")]
    total_count: u64,
    nodes: Vec<GraphQlThreadNode>,
}

#[derive(Debug, Deserialize)]
struct GraphQlThreadNode {
    id: String,
    #[serde(rename = "isResolved")]
    is_resolved: bool,
    #[serde(rename = "isOutdated")]
    is_outdated: bool,
    #[serde(default)]
    path: Option<String>,
    // No `typename` is read from `resolvedBy`: the field is GraphQL-typed
    // `User` regardless of the resolving actor's real attestation (design
    // §3.4/§6), so testing it here would test an artifact of the query's
    // static type rather than the actor. `GraphQlActorMirror`'s `typename`
    // is simply left unused for this field.
    #[serde(default, rename = "resolvedBy")]
    resolved_by: Option<GraphQlActorMirror>,
    comments: GraphQlThreadCommentsConnection,
}

#[derive(Debug, Deserialize)]
struct GraphQlThreadCommentsConnection {
    #[serde(rename = "totalCount")]
    total_count: u64,
    nodes: Vec<GraphQlThreadCommentNode>,
}

#[derive(Debug, Deserialize)]
struct GraphQlThreadCommentNode {
    body: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(default, rename = "lastEditedAt")]
    last_edited_at: Option<String>,
    url: String,
    #[serde(rename = "authorAssociation")]
    author_association: String,
    author: GraphQlActorMirror,
}

type GraphQlCommitsEnvelope = GraphQlEnvelope<GraphQlCommitsPr>;

#[derive(Debug, Deserialize)]
struct GraphQlCommitsPr {
    commits: GraphQlCommitsConnection,
}

#[derive(Debug, Deserialize)]
struct GraphQlCommitsConnection {
    nodes: Vec<GraphQlCommitConnectionNode>,
}

#[derive(Debug, Deserialize)]
struct GraphQlCommitConnectionNode {
    commit: GraphQlCommitDetail,
}

#[derive(Debug, Deserialize)]
struct GraphQlCommitDetail {
    author: GraphQlCommitActorWrapper,
    committer: GraphQlCommitActorWrapper,
}

#[derive(Debug, Deserialize)]
struct GraphQlCommitActorWrapper {
    #[serde(default)]
    user: Option<GraphQlUserActorMirror>,
}

/// Like `GraphQlActorMirror`, but `id` missing here is a hard refusal, not
/// an `unattributed` classification — this feeds the implementation
/// actor-id set (design §6).
#[derive(Debug, Deserialize)]
struct GraphQlUserActorMirror {
    login: String,
    #[serde(default)]
    id: Option<String>,
}

type GraphQlChecksEnvelope = GraphQlEnvelope<GraphQlChecksPr>;

#[derive(Debug, Deserialize)]
struct GraphQlChecksPr {
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
    commits: GraphQlChecksCommitsConnection,
}

#[derive(Debug, Deserialize)]
struct GraphQlChecksCommitsConnection {
    nodes: Vec<GraphQlChecksCommitNode>,
}

#[derive(Debug, Deserialize)]
struct GraphQlChecksCommitNode {
    commit: GraphQlChecksCommitDetail,
}

#[derive(Debug, Deserialize)]
struct GraphQlChecksCommitDetail {
    oid: String,
    #[serde(default, rename = "statusCheckRollup")]
    status_check_rollup: Option<GraphQlStatusCheckRollup>,
}

#[derive(Debug, Deserialize)]
struct GraphQlStatusCheckRollup {
    contexts: GraphQlContextsConnection,
}

/// Heterogeneous `CheckRun`/`StatusContext` union — parsed generically and
/// dispatched on `__typename` by `build_check_evidence` rather than an
/// internally tagged enum, since the two variants share no field names.
#[derive(Debug, Deserialize)]
struct GraphQlContextsConnection {
    nodes: Vec<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn unique_scratch_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "casegraphen-github-evidence-test-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn write_json(dir: &Path, name: &str, value: &Value) -> (String, String) {
        let bytes = serde_json::to_vec(value).expect("value serializes");
        fs::write(dir.join(name), &bytes).expect("write fixture");
        (name.to_owned(), format!("sha256:{}", sha256_hex(&bytes)))
    }

    fn entry(category: CaptureCategory, artifact_path: &str, content_hash: &str) -> CaptureEntry {
        CaptureEntry {
            category,
            issue_number: None,
            artifact_path: artifact_path.to_owned(),
            content_hash: content_hash.to_owned(),
            command_record: Vec::new(),
        }
    }

    const REPO: &str = "OWNER/repo";
    const BASE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HEAD_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const CAPTURED_AT: &str = "2026-01-01T00:00:00Z";

    fn pr_value() -> Value {
        json!({
            "number": 7,
            "title": "Add a thing",
            "url": "https://github.com/OWNER/repo/pull/7",
            "state": "MERGED",
            "author": {"id": "actor:pr-author", "login": "alice"},
            "baseRefName": "main",
            "baseRefOid": BASE_SHA,
            "headRefName": "feature",
            "headRefOid": HEAD_SHA,
            "createdAt": "2026-01-01T00:00:00Z",
            "mergedAt": "2026-01-01T01:00:00Z",
            "mergeable": "UNKNOWN",
            "mergeStateStatus": "UNKNOWN",
            "body": "pr body"
        })
    }

    fn files_value() -> Value {
        json!({
            "files": [
                {"path": "b.rs", "additions": 2, "deletions": 1, "changeType": "MODIFIED"},
                {"path": "a.rs", "additions": 1, "deletions": 0, "changeType": "ADDED"}
            ]
        })
    }

    fn reviews_value() -> Value {
        json!({
            "data": {"repository": {"pullRequest": {
                "number": 7,
                "reviews": {"totalCount": 1, "nodes": [
                    {
                        "state": "APPROVED",
                        "body": "lgtm",
                        "submittedAt": "2026-01-01T02:00:00Z",
                        "lastEditedAt": null,
                        "url": "https://github.com/OWNER/repo/pull/7#pullrequestreview-1",
                        "authorAssociation": "MEMBER",
                        "commit": {"oid": HEAD_SHA},
                        "author": {"__typename": "User", "login": "alice", "id": "actor:pr-author"}
                    }
                ]}
            }}}
        })
    }

    fn review_threads_value() -> Value {
        json!({
            "data": {"repository": {"pullRequest": {
                "number": 7,
                "baseRefOid": BASE_SHA,
                "headRefOid": HEAD_SHA,
                "reviewThreads": {"totalCount": 1, "nodes": [
                    {
                        "id": "thread-1",
                        "isResolved": true,
                        "isOutdated": false,
                        "path": "a.rs",
                        "resolvedBy": {"__typename": "User", "login": "alice", "id": "actor:pr-author"},
                        "comments": {"totalCount": 1, "nodes": [
                            {
                                "body": "please fix",
                                "createdAt": "2026-01-01T02:30:00Z",
                                "lastEditedAt": null,
                                "url": "https://github.com/OWNER/repo/pull/7#discussion_r1",
                                "authorAssociation": "NONE",
                                "author": {"__typename": "Bot", "login": "reviewbot", "id": "actor:bot-1"}
                            }
                        ]}
                    }
                ]}
            }}}
        })
    }

    fn commits_value() -> Value {
        json!({
            "data": {"repository": {"pullRequest": {
                "commits": {"nodes": [
                    {"commit": {
                        "author": {"user": {"__typename": "User", "login": "alice", "id": "actor:pr-author"}},
                        "committer": {"user": {"__typename": "User", "login": "alice", "id": "actor:pr-author"}}
                    }}
                ]}
            }}}
        })
    }

    fn checks_value() -> Value {
        json!({
            "data": {"repository": {"pullRequest": {
                "headRefOid": HEAD_SHA,
                "commits": {"nodes": [
                    {"commit": {
                        "oid": HEAD_SHA,
                        "statusCheckRollup": {"contexts": {"nodes": [
                            {
                                "__typename": "CheckRun",
                                "name": "quality",
                                "status": "COMPLETED",
                                "conclusion": "SUCCESS",
                                "startedAt": "2026-01-01T03:00:00Z",
                                "completedAt": "2026-01-01T03:05:00Z",
                                "detailsUrl": "https://ci.example/1",
                                "checkSuite": {"app": {"slug": "github-actions"}, "workflowRun": {"workflow": {"name": "Quality"}}}
                            }
                        ]}}
                    }}
                ]}
            }}}
        })
    }

    /// Writes a baseline capture (one artifact per required category, no
    /// issues) and returns its manifest. Individual tests mutate a copy of
    /// one JSON value and re-derive that entry's hash to exercise a single
    /// refusal or normalization path without rebuilding the whole fixture.
    fn write_baseline_capture(dir: &Path) -> CaptureManifest {
        write_capture(
            dir,
            &pr_value(),
            &files_value(),
            &reviews_value(),
            &review_threads_value(),
            &commits_value(),
            &checks_value(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write_capture(
        dir: &Path,
        pr: &Value,
        files: &Value,
        reviews: &Value,
        review_threads: &Value,
        commits: &Value,
        checks: &Value,
    ) -> CaptureManifest {
        let (pr_path, pr_hash) = write_json(dir, "pr.json", pr);
        let (files_path, files_hash) = write_json(dir, "files.json", files);
        let (reviews_path, reviews_hash) = write_json(dir, "reviews.json", reviews);
        let (threads_path, threads_hash) = write_json(dir, "review_threads.json", review_threads);
        let (commits_path, commits_hash) = write_json(dir, "commits.json", commits);
        let (checks_path, checks_hash) = write_json(dir, "checks.json", checks);
        CaptureManifest {
            schema: GITHUB_CAPTURE_MANIFEST_SCHEMA.to_owned(),
            repository: REPO.to_owned(),
            issue_numbers: Vec::new(),
            pr_number: 7,
            captured_at: CAPTURED_AT.to_owned(),
            capture_tool: "gh".to_owned(),
            entries: vec![
                entry(CaptureCategory::Pr, &pr_path, &pr_hash),
                entry(CaptureCategory::Files, &files_path, &files_hash),
                entry(CaptureCategory::Reviews, &reviews_path, &reviews_hash),
                entry(CaptureCategory::ReviewThreads, &threads_path, &threads_hash),
                entry(CaptureCategory::Commits, &commits_path, &commits_hash),
                entry(CaptureCategory::Checks, &checks_path, &checks_hash),
            ],
        }
    }

    #[test]
    fn hash_mismatch_is_a_hard_refusal() {
        let dir = unique_scratch_dir("hash-mismatch");
        fs::create_dir_all(&dir).expect("create dir");
        let mut manifest = write_baseline_capture(&dir);
        manifest.entries[0].content_hash =
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned();

        let error = normalize(&manifest, &dir).expect_err("mismatched hash must refuse");
        assert_eq!(error.code, "content_hash_mismatch");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn artifact_path_escape_is_a_hard_refusal() {
        let dir = unique_scratch_dir("path-escape");
        fs::create_dir_all(&dir).expect("create dir");
        let mut manifest = write_baseline_capture(&dir);
        manifest.entries[0].artifact_path = "../outside.json".to_owned();

        let error = normalize(&manifest, &dir).expect_err("a `..` component must refuse");
        assert_eq!(error.code, "artifact_path_escape");

        let mut manifest = write_baseline_capture(&dir);
        manifest.entries[0].artifact_path = "/etc/passwd".to_owned();
        let error = normalize(&manifest, &dir).expect_err("an absolute path must refuse");
        assert_eq!(error.code, "artifact_path_escape");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_required_category_is_a_hard_refusal() {
        let dir = unique_scratch_dir("missing-category");
        fs::create_dir_all(&dir).expect("create dir");
        let mut manifest = write_baseline_capture(&dir);
        manifest
            .entries
            .retain(|entry| entry.category != CaptureCategory::Checks);

        let error = normalize(&manifest, &dir).expect_err("a missing category must refuse");
        // Caught by the upfront shape check (every required category must
        // appear exactly once) before `one_of` ever runs — `one_of`'s own
        // "missing" refusal exists for defense in depth, not as the code
        // this path actually returns.
        assert_eq!(error.code, "invalid_category_count");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn actor_set_source_missing_node_id_is_a_hard_refusal() {
        let dir = unique_scratch_dir("actor-set-missing-id");
        fs::create_dir_all(&dir).expect("create dir");
        let commits_without_id = json!({
            "data": {"repository": {"pullRequest": {
                "commits": {"nodes": [
                    {"commit": {
                        "author": {"user": {"login": "alice"}},
                        "committer": {"user": {"login": "alice"}}
                    }}
                ]}
            }}}
        });
        let manifest = write_capture(
            &dir,
            &pr_value(),
            &files_value(),
            &reviews_value(),
            &review_threads_value(),
            &commits_without_id,
            &checks_value(),
        );

        let error =
            normalize(&manifest, &dir).expect_err("a commits actor without a node id must refuse");
        assert_eq!(error.code, "actor_set_source_missing_id");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finding_author_missing_attestation_normalizes_without_refusal() {
        let dir = unique_scratch_dir("finding-unattributed");
        fs::create_dir_all(&dir).expect("create dir");
        // The real corpus's own unattested shape: a `gh --json` review whose
        // author is `{"login": …}` only (design's "missing actor
        // attestation" fixture).
        let reviews_without_attestation = json!({
            "data": {"repository": {"pullRequest": {
                "number": 7,
                "reviews": {"totalCount": 1, "nodes": [
                    {
                        "state": "APPROVED",
                        "body": "lgtm",
                        "submittedAt": "2026-01-01T02:00:00Z",
                        "url": "https://github.com/OWNER/repo/pull/7#pullrequestreview-9",
                        "authorAssociation": "NONE",
                        "commit": {"oid": HEAD_SHA},
                        "author": {"login": "deleted-account"}
                    }
                ]}
            }}}
        });
        let manifest = write_capture(
            &dir,
            &pr_value(),
            &files_value(),
            &reviews_without_attestation,
            &review_threads_value(),
            &commits_value(),
            &checks_value(),
        );

        let normalized =
            normalize(&manifest, &dir).expect("missing finding attestation must not refuse");
        let unattributed = normalized
            .review_findings
            .iter()
            .find(|finding| finding.author.login == "deleted-account")
            .expect("the unattested review is present");
        assert_eq!(unattributed.author.id, None);
        assert_eq!(unattributed.author.typename, None);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn intra_capture_head_disagreement_is_a_hard_refusal() {
        let dir = unique_scratch_dir("intra-capture-mismatch");
        fs::create_dir_all(&dir).expect("create dir");
        let mut straddled_threads = review_threads_value();
        straddled_threads["data"]["repository"]["pullRequest"]["headRefOid"] =
            json!("cccccccccccccccccccccccccccccccccccccccc");
        let manifest = write_capture(
            &dir,
            &pr_value(),
            &files_value(),
            &reviews_value(),
            &straddled_threads,
            &commits_value(),
            &checks_value(),
        );

        let error =
            normalize(&manifest, &dir).expect_err("a capture that straddled a push must refuse");
        assert_eq!(error.code, "intra_capture_head_mismatch");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_findings_collapse_and_unattributed_findings_never_collapse() {
        let dir = unique_scratch_dir("duplicate-collapse");
        fs::create_dir_all(&dir).expect("create dir");
        let mut threads = review_threads_value();
        threads["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"][0]["comments"] = json!({
            "totalCount": 4,
            "nodes": [
                {
                    "body": "duplicate nitpick",
                    "createdAt": "2026-01-01T02:30:00Z",
                    "url": "https://github.com/OWNER/repo/pull/7#discussion_r1",
                    "authorAssociation": "NONE",
                    "author": {"__typename": "Bot", "login": "reviewbot", "id": "actor:bot-1"}
                },
                {
                    "body": "duplicate nitpick",
                    "createdAt": "2026-01-01T02:31:00Z",
                    "url": "https://github.com/OWNER/repo/pull/7#discussion_r2",
                    "authorAssociation": "NONE",
                    "author": {"__typename": "Bot", "login": "reviewbot", "id": "actor:bot-1"}
                },
                {
                    "body": "unattributed duplicate",
                    "createdAt": "2026-01-01T02:32:00Z",
                    "url": "https://github.com/OWNER/repo/pull/7#discussion_r3",
                    "authorAssociation": "NONE",
                    "author": {"login": "deleted-account"}
                },
                {
                    "body": "unattributed duplicate",
                    "createdAt": "2026-01-01T02:33:00Z",
                    "url": "https://github.com/OWNER/repo/pull/7#discussion_r4",
                    "authorAssociation": "NONE",
                    "author": {"login": "deleted-account"}
                }
            ]
        });
        let manifest = write_capture(
            &dir,
            &pr_value(),
            &files_value(),
            &reviews_value(),
            &threads,
            &commits_value(),
            &checks_value(),
        );

        let normalized = normalize(&manifest, &dir).expect("normalize");
        let bot_findings: Vec<_> = normalized
            .review_findings
            .iter()
            .filter(|finding| finding.author.id.as_deref() == Some("actor:bot-1"))
            .collect();
        assert_eq!(
            bot_findings.len(),
            1,
            "the two identical bot comments must collapse into one"
        );
        assert_eq!(bot_findings[0].duplicate_count, 2);
        assert!(
            bot_findings[0].actionable,
            "the earlier (opening) comment must survive collapse as the representative"
        );

        let unattributed_findings: Vec<_> = normalized
            .review_findings
            .iter()
            .filter(|finding| finding.author.login == "deleted-account")
            .collect();
        assert_eq!(
            unattributed_findings.len(),
            2,
            "findings without an author id must never collapse, even when textually identical"
        );
        assert!(unattributed_findings
            .iter()
            .all(|finding| finding.duplicate_count == 1));

        fs::remove_dir_all(&dir).ok();
    }

    /// The defect the collapse key had before `review_state`/`commit_sha`
    /// joined it: an empty body is the *common* case for a real approval
    /// (18 of the real PR-101 corpus's 20 reviews have one), so
    /// `(author.id, body_content_hash, path)` alone would merge an
    /// `APPROVED` review bound to an old commit with a `COMMENTED` review at
    /// head from the same outside reviewer — silently gaining or losing
    /// exactly the signal `evaluate_independence`'s `commit_sha ==
    /// head.sha` check exists to gate. These must never collapse.
    #[test]
    fn same_author_same_empty_body_reviews_at_different_states_and_commits_never_collapse() {
        let dir = unique_scratch_dir("no-collapse-across-state-and-commit");
        fs::create_dir_all(&dir).expect("create dir");
        let older_sha = "5555555555555555555555555555555555555555";
        let reviews_with_a_stale_approval = json!({
            "data": {"repository": {"pullRequest": {
                "number": 7,
                "reviews": {"totalCount": 2, "nodes": [
                    {
                        "state": "APPROVED",
                        "body": "",
                        "submittedAt": "2026-01-01T02:00:00Z",
                        "url": "https://github.com/OWNER/repo/pull/7#pullrequestreview-1",
                        "authorAssociation": "NONE",
                        "commit": {"oid": older_sha},
                        "author": {"__typename": "User", "login": "carol", "id": "actor:outside-reviewer"}
                    },
                    {
                        "state": "COMMENTED",
                        "body": "",
                        "submittedAt": "2026-01-01T02:10:00Z",
                        "url": "https://github.com/OWNER/repo/pull/7#pullrequestreview-2",
                        "authorAssociation": "NONE",
                        "commit": {"oid": HEAD_SHA},
                        "author": {"__typename": "User", "login": "carol", "id": "actor:outside-reviewer"}
                    }
                ]}
            }}}
        });
        let manifest = write_capture(
            &dir,
            &pr_value(),
            &files_value(),
            &reviews_with_a_stale_approval,
            &review_threads_value(),
            &commits_value(),
            &checks_value(),
        );

        let normalized = normalize(&manifest, &dir).expect("normalize");
        let carol_findings: Vec<_> = normalized
            .review_findings
            .iter()
            .filter(|finding| finding.author.id.as_deref() == Some("actor:outside-reviewer"))
            .collect();
        assert_eq!(
            carol_findings.len(),
            2,
            "an empty-body APPROVED at an old commit and an empty-body COMMENTED at head, from \
             the same author, must remain two distinct findings, not one"
        );
        assert!(carol_findings
            .iter()
            .all(|finding| finding.duplicate_count == 1));
        assert!(carol_findings
            .iter()
            .any(
                |finding| finding.review_state.as_deref() == Some("APPROVED")
                    && finding.commit_sha.as_deref() == Some(older_sha)
            ));
        assert!(carol_findings
            .iter()
            .any(
                |finding| finding.review_state.as_deref() == Some("COMMENTED")
                    && finding.commit_sha.as_deref() == Some(HEAD_SHA)
            ));

        fs::remove_dir_all(&dir).ok();
    }

    /// The other half of the same defect: without `thread_id` and the
    /// thread's `resolved` flag in the key, two identical bot comments
    /// sitting in two *different* threads would collapse into one finding,
    /// discarding the fact that they are two separate obligations — and if
    /// the threads disagree on resolution, discarding which one is still
    /// open.
    #[test]
    fn identical_findings_in_different_threads_never_collapse() {
        let dir = unique_scratch_dir("no-collapse-across-threads");
        fs::create_dir_all(&dir).expect("create dir");
        let two_threads_with_identical_comments = json!({
            "data": {"repository": {"pullRequest": {
                "number": 7,
                "baseRefOid": BASE_SHA,
                "headRefOid": HEAD_SHA,
                "reviewThreads": {"totalCount": 2, "nodes": [
                    {
                        "id": "thread-1",
                        "isResolved": true,
                        "isOutdated": false,
                        "path": "a.rs",
                        "comments": {"totalCount": 1, "nodes": [
                            {
                                "body": "same nitpick",
                                "createdAt": "2026-01-01T02:30:00Z",
                                "url": "https://github.com/OWNER/repo/pull/7#discussion_r1",
                                "authorAssociation": "NONE",
                                "author": {"__typename": "Bot", "login": "reviewbot", "id": "actor:bot-1"}
                            }
                        ]}
                    },
                    {
                        "id": "thread-2",
                        "isResolved": false,
                        "isOutdated": false,
                        "path": "a.rs",
                        "comments": {"totalCount": 1, "nodes": [
                            {
                                "body": "same nitpick",
                                "createdAt": "2026-01-01T02:31:00Z",
                                "url": "https://github.com/OWNER/repo/pull/7#discussion_r2",
                                "authorAssociation": "NONE",
                                "author": {"__typename": "Bot", "login": "reviewbot", "id": "actor:bot-1"}
                            }
                        ]}
                    }
                ]}
            }}}
        });
        let manifest = write_capture(
            &dir,
            &pr_value(),
            &files_value(),
            &reviews_value(),
            &two_threads_with_identical_comments,
            &commits_value(),
            &checks_value(),
        );

        let normalized = normalize(&manifest, &dir).expect("normalize");
        let bot_findings: Vec<_> = normalized
            .review_findings
            .iter()
            .filter(|finding| finding.author.id.as_deref() == Some("actor:bot-1"))
            .collect();
        assert_eq!(
            bot_findings.len(),
            2,
            "identical comments in two different threads are two obligations, not one \
             collapsed finding"
        );
        assert!(bot_findings
            .iter()
            .all(|finding| finding.duplicate_count == 1));
        let resolved_states: BTreeSet<bool> = bot_findings
            .iter()
            .map(|finding| finding.thread.as_ref().expect("thread state").resolved)
            .collect();
        assert_eq!(
            resolved_states,
            BTreeSet::from([true, false]),
            "the two threads' differing resolution state must survive — collapsing would have \
             erased which obligation is still open"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// `capture_totals` must reflect a real gap between what the provider
    /// claims (`totalCount`) and what this capture actually received
    /// (`nodes.len()`) — the bookkeeping T4 needs to derive
    /// `threads_truncated`/`files_truncated` from this output alone,
    /// without re-parsing the raw artifacts itself.
    #[test]
    fn capture_totals_reports_a_real_truncation_gap() {
        let dir = unique_scratch_dir("truncation-bookkeeping");
        fs::create_dir_all(&dir).expect("create dir");
        let truncated_reviews = json!({
            "data": {"repository": {"pullRequest": {
                "number": 7,
                // Provider claims 5 reviews exist; only 1 node was captured.
                "reviews": {"totalCount": 5, "nodes": [
                    {
                        "state": "APPROVED",
                        "body": "lgtm",
                        "submittedAt": "2026-01-01T02:00:00Z",
                        "url": "https://github.com/OWNER/repo/pull/7#pullrequestreview-1",
                        "authorAssociation": "MEMBER",
                        "commit": {"oid": HEAD_SHA},
                        "author": {"__typename": "User", "login": "alice", "id": "actor:pr-author"}
                    }
                ]}
            }}}
        });
        let truncated_threads = json!({
            "data": {"repository": {"pullRequest": {
                "number": 7,
                "baseRefOid": BASE_SHA,
                "headRefOid": HEAD_SHA,
                // Provider claims 4 threads exist; only 1 node was captured,
                // and that thread's own comment count is likewise truncated.
                "reviewThreads": {"totalCount": 4, "nodes": [
                    {
                        "id": "thread-1",
                        "isResolved": true,
                        "isOutdated": false,
                        "path": "a.rs",
                        "comments": {"totalCount": 6, "nodes": [
                            {
                                "body": "please fix",
                                "createdAt": "2026-01-01T02:30:00Z",
                                "url": "https://github.com/OWNER/repo/pull/7#discussion_r1",
                                "authorAssociation": "NONE",
                                "author": {"__typename": "Bot", "login": "reviewbot", "id": "actor:bot-1"}
                            }
                        ]}
                    }
                ]}
            }}}
        });
        let manifest = write_capture(
            &dir,
            &pr_value(),
            &files_value(),
            &truncated_reviews,
            &truncated_threads,
            &commits_value(),
            &checks_value(),
        );

        let normalized = normalize(&manifest, &dir).expect("normalize");
        let totals = &normalized.capture_totals;
        assert_eq!(totals.reviews_reported_total, 5);
        assert_eq!(totals.reviews_received, 1);
        assert_eq!(totals.review_threads_reported_total, 4);
        assert_eq!(totals.review_threads_received, 1);
        assert_eq!(totals.thread_comment_totals.len(), 1);
        assert_eq!(totals.thread_comment_totals[0].thread_id, "thread-1");
        assert_eq!(totals.thread_comment_totals[0].reported_total, 6);
        assert_eq!(totals.thread_comment_totals[0].received, 1);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn normalizing_the_same_bytes_twice_is_byte_identical() {
        let dir = unique_scratch_dir("determinism");
        fs::create_dir_all(&dir).expect("create dir");
        let manifest = write_baseline_capture(&dir);

        let first = normalize(&manifest, &dir).expect("first normalize");
        let second = normalize(&manifest, &dir).expect("second normalize");

        assert_eq!(first, second);
        assert_eq!(
            serde_json::to_vec(&first.pr_observation).expect("serialize"),
            serde_json::to_vec(&second.pr_observation).expect("serialize"),
        );
        assert_eq!(
            first.pr_observation.normalized_content_hash,
            second.pr_observation.normalized_content_hash
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// The manifest's own `entries` array is the one "raw JSON array order"
    /// normalize can be invariant to without changing what was captured:
    /// permuting the *content* of a provider array (e.g. `reviews.nodes`)
    /// changes that artifact's bytes, hence its `content_hash`, hence its
    /// `source_record_id` — a genuinely different capture, not a
    /// same-capture reordering. `manifest.entries` is different: it is a
    /// caller-authored list of which artifact serves which category, its
    /// order carries no meaning (`one_of` looks entries up by category, not
    /// position), and every artifact's bytes stay fixed across variants.
    #[test]
    fn permuting_manifest_entry_order_never_changes_the_normalized_content_hash() {
        let dir = unique_scratch_dir("permute-entries");
        fs::create_dir_all(&dir).expect("create dir");
        let baseline_manifest = write_baseline_capture(&dir);
        let baseline_hash = normalize(&baseline_manifest, &dir)
            .expect("normalize baseline")
            .pr_observation
            .normalized_content_hash;

        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let mut entries = baseline_manifest.entries.clone();
                // Fisher-Yates driven by the fuzzer's own byte stream — the
                // same "derive a shuffle from arbitrary bytes" idiom
                // `native_hash.rs`'s own arbtest property test uses.
                for i in (1..entries.len()).rev() {
                    let selector: usize = u.arbitrary()?;
                    entries.swap(i, selector % (i + 1));
                }
                let mut manifest = baseline_manifest.clone();
                manifest.entries = entries;

                let hash = normalize(&manifest, &dir)
                    .expect("normalize permuted manifest")
                    .pr_observation
                    .normalized_content_hash;
                assert_eq!(hash, baseline_hash);
                Ok(())
            },
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_pilot_corpus_normalizes_to_the_documented_ground_truth() {
        let capture_dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/pilots/issue-102/source");
        let read = |name: &str| {
            fs::read(capture_dir.join(name))
                .unwrap_or_else(|error| panic!("read pilot fixture {name}: {error}"))
        };
        let hash_of = |name: &str| format!("sha256:{}", sha256_hex(&read(name)));

        // Cross-checks the dynamically computed hash against the design
        // doc's documented ground truth (§10.1), so a drifted retained
        // fixture fails loudly here rather than only inside a distant CLI
        // integration test.
        assert_eq!(
            hash_of("issue-92.json"),
            "sha256:e41b2147adbaf76470bba4a14ede3ed816e09dd0568b486a9690c40de1bdd355"
        );
        assert_eq!(
            hash_of("pr-101.json"),
            "sha256:07ac47fc5a0c2420ee5f5bb500001e44b6227638cfd2bc59f3e916ef2920ca26"
        );
        assert_eq!(
            hash_of("pr-101-reviews.json"),
            "sha256:ed396eff46cbe15ee2140abf3cc916a2febecd4e0821c8b7a8f1b8752b696348"
        );
        assert_eq!(
            hash_of("pr-101-threads.json"),
            "sha256:03229662f2b7e1b327b5d8d8ef76d7e01d1fd5b883d52d6ea624dc81cff5d918"
        );
        assert_eq!(
            hash_of("pr-101-commits.json"),
            "sha256:a597a0bc8c8682401abe0318c3750985c57570145bea493718b51cb34e8fc36b"
        );
        assert_eq!(
            hash_of("pr-101-checks.json"),
            "sha256:216cf7c74bb3a3992d89e8e195a47af9c18cb0f0ac63cdf9c439b771ceb6a82c"
        );

        let manifest = CaptureManifest {
            schema: GITHUB_CAPTURE_MANIFEST_SCHEMA.to_owned(),
            repository: "CAPHTECH/casegraphen".to_owned(),
            issue_numbers: vec![92],
            pr_number: 101,
            captured_at: "2026-08-06T09:00:00Z".to_owned(),
            capture_tool: "gh".to_owned(),
            entries: vec![
                CaptureEntry {
                    category: CaptureCategory::Issue,
                    issue_number: Some(92),
                    artifact_path: "issue-92.json".to_owned(),
                    content_hash: hash_of("issue-92.json"),
                    command_record: Vec::new(),
                },
                entry(CaptureCategory::Pr, "pr-101.json", &hash_of("pr-101.json")),
                entry(
                    CaptureCategory::Files,
                    "pr-101.json",
                    &hash_of("pr-101.json"),
                ),
                entry(
                    CaptureCategory::Reviews,
                    "pr-101-reviews.json",
                    &hash_of("pr-101-reviews.json"),
                ),
                entry(
                    CaptureCategory::ReviewThreads,
                    "pr-101-threads.json",
                    &hash_of("pr-101-threads.json"),
                ),
                entry(
                    CaptureCategory::Commits,
                    "pr-101-commits.json",
                    &hash_of("pr-101-commits.json"),
                ),
                entry(
                    CaptureCategory::Checks,
                    "pr-101-checks.json",
                    &hash_of("pr-101-checks.json"),
                ),
            ],
        };

        let normalized = normalize(&manifest, &capture_dir).expect("pilot corpus normalizes");
        let observation = &normalized.pr_observation;

        assert_eq!(observation.repository, "CAPHTECH/casegraphen");
        assert_eq!(observation.pr.number, 101);
        assert_eq!(
            observation.base.sha,
            "947f347f219a60775bcf71b226ce778cc8ea21f4"
        );
        assert_eq!(
            observation.head.sha,
            "c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b"
        );
        assert_eq!(observation.liveness.state, "MERGED");
        assert_eq!(observation.liveness.mergeable, MergeableState::Unknown);
        assert_eq!(observation.changed_files.len(), 78);
        assert_eq!(
            observation.implementation_actors.actor_ids,
            vec!["MDQ6VXNlcjc5MDUxMQ=="]
        );
        assert_eq!(observation.issues.len(), 1);
        assert_eq!(observation.issues[0].number, 92);
        assert_eq!(observation.issues[0].state, "CLOSED");
        assert_eq!(observation.issues[0].closed_by_pr_numbers, vec![101]);

        assert_eq!(normalized.check_evidence.len(), 3);
        let check_runs = normalized
            .check_evidence
            .iter()
            .filter(|check| check.kind == CheckKind::CheckRun)
            .count();
        assert_eq!(check_runs, 2);
        let status_context = normalized
            .check_evidence
            .iter()
            .find(|check| check.kind == CheckKind::StatusContext)
            .expect("one status context");
        assert_eq!(status_context.name, "CodeRabbit");
        assert_eq!(status_context.state.as_deref(), Some("SUCCESS"));
        assert_eq!(
            status_context.description.as_deref(),
            Some("Review rate limited")
        );
        assert_eq!(
            status_context.creator,
            Some(Actor {
                id: "BOT_kgDOCCSy2w".to_owned(),
                login: "coderabbitai".to_owned(),
                typename: "Bot".to_owned(),
            })
        );

        let thread_findings: Vec<_> = normalized
            .review_findings
            .iter()
            .filter(|finding| finding.kind == FindingKind::ThreadComment)
            .collect();
        let distinct_threads: BTreeSet<_> = thread_findings
            .iter()
            .map(|finding| {
                finding
                    .thread
                    .as_ref()
                    .expect("thread comments carry thread state")
                    .thread_id
                    .clone()
            })
            .collect();
        assert_eq!(distinct_threads.len(), 9);
        assert!(thread_findings.iter().all(|finding| finding
            .thread
            .as_ref()
            .expect("thread state")
            .resolved));
        let unresolved_actionable = thread_findings
            .iter()
            .filter(|finding| {
                finding.actionable && !finding.thread.as_ref().expect("thread state").resolved
            })
            .count();
        assert_eq!(unresolved_actionable, 0);

        // 20 reviews collapse to 4 distinct (author.id, body_content_hash)
        // groups on the real corpus: 9 of coderabbitai's and 9 of
        // rizumita's reviews share an empty body verbatim, plus one
        // non-empty review from each — real dogfood proof that duplicate
        // collapse applies to review summaries, not only thread comments
        // (design §3.4's key is not scoped to one finding kind).
        let review_summaries: Vec<_> = normalized
            .review_findings
            .iter()
            .filter(|finding| finding.kind == FindingKind::ReviewSummary)
            .collect();
        assert_eq!(review_summaries.len(), 4);
        let mut duplicate_counts: Vec<u32> = review_summaries
            .iter()
            .map(|finding| finding.duplicate_count)
            .collect();
        duplicate_counts.sort_unstable();
        assert_eq!(duplicate_counts, vec![1, 1, 9, 9]);
        assert_eq!(
            duplicate_counts.iter().sum::<u32>(),
            20,
            "duplicate_count must preserve the full pre-collapse review count"
        );
        assert!(review_summaries
            .iter()
            .all(|finding| finding.review_state.as_deref() == Some("COMMENTED")));

        assert!(normalized.domain_findings.is_empty());

        // capture_totals: nothing truncated in the pilot capture — every
        // reported total equals what was actually received, on the real
        // corpus (20 reviews, 9 threads, 3 comments per thread).
        let totals = &normalized.capture_totals;
        assert_eq!(totals.reviews_reported_total, 20);
        assert_eq!(totals.reviews_received, 20);
        assert_eq!(totals.review_threads_reported_total, 9);
        assert_eq!(totals.review_threads_received, 9);
        assert_eq!(totals.thread_comment_totals.len(), 9);
        assert!(totals
            .thread_comment_totals
            .iter()
            .all(|thread| thread.reported_total == 3 && thread.received == 3));
    }
}
