//! `github observe|refresh|project` — the store-free, read-only CLI surface
//! over `src/github_evidence/` (issue #102, design doc §9).
//!
//! All three operations share one shape with `memory query`/`memory check`
//! (`native_cli/ops/memory.rs`): they take no `--store`, never open a
//! `NativeCaseStore`, and every output record carries `accepted: false` and
//! `mutation_performed: false`. Integrity failures (manifest hash mismatch,
//! path escape, missing category, intra-capture disagreement, a declared
//! `--previous-observation` basis that disagrees with the retained bytes) are
//! hard errors via `NativeCliError::Memory`, the same disposition
//! `memory_propose` uses for a pre-store validation refusal. `stale_head` and
//! an unmet `--require-independent-review` are domain findings — successful
//! results carrying an obstruction, the same exit discipline `memory check`
//! uses for an invalid claim.

use super::io::{parse_strict, read_json};
use super::{report, NativeCliError, NativeCommandResult};
use crate::github_evidence::{
    classify_refresh, evaluate_independence, normalize, project_review, CaptureManifest,
    NormalizedCapture, PrObservation, RefreshDisposition, GITHUB_CAPTURE_MANIFEST_SCHEMA,
    GITHUB_PR_OBSERVATION_SCHEMA,
};
use crate::memory::MemoryValidationFinding;
use serde_json::{json, Value};
use std::path::Path;

pub(in crate::native_cli) fn github_observe(
    manifest: &Path,
    capture_dir: &Path,
) -> Result<NativeCommandResult<Value>, NativeCliError> {
    let capture = normalize_capture(&read_manifest(manifest)?, capture_dir)?;
    // `observe` classifies roles but declares no independent-review demand —
    // that policy question belongs only to `project`'s `--require-independent-review`
    // flag (design §9). `false` here makes `policy.satisfied` vacuous rather
    // than false, so `observe`'s own domain-finding disposition is driven
    // only by the normalize-time domain findings below, not by a policy this
    // command never asked about.
    let independence = evaluate_independence(
        &capture.pr_observation,
        &capture.review_findings,
        &capture.check_evidence,
        false,
    );
    let domain_finding = !capture.domain_findings.is_empty();
    Ok(NativeCommandResult::with_domain_finding(
        report(
            "casegraphen github observe",
            json!({
                "source_records": capture.source_records,
                "pr_observation": capture.pr_observation,
                "check_evidence": capture.check_evidence,
                "review_findings": capture.review_findings,
                "independence": independence,
                "domain_findings": capture.domain_findings,
                "accepted": false,
                "mutation_performed": false
            }),
        ),
        domain_finding,
    ))
}

/// `--previous-manifest`/`--previous-capture-dir` supply the previous review
/// basis as a **capture**, re-normalized with the exact same `normalize()`
/// every other observation goes through, and `previous_checks`/
/// `previous_findings` come from that re-normalization — `PrObservation`
/// alone carries no per-check or per-finding state to diff against (a design
/// gap T3 found; this is the CLI-side resolution, stronger than the design
/// sketch because it needs no new record-as-input surface: see the design
/// doc §7/§9 and `IMPLEMENTATION-PLAN.md`'s T5 section for the ruling).
///
/// `--previous-observation` stays optional and is the operator's *declared*
/// review basis (design §6.1): when supplied, it must equal the
/// re-normalized previous capture's own `pr_observation` byte-for-byte, or
/// this refuses before `classify_refresh` ever runs — a declared basis that
/// disagrees with the retained bytes is an integrity failure, not a drift.
/// `classify_refresh` itself still recomputes and checks whatever
/// `PrObservation` it is given against its own claimed hash (the cheaper
/// first gate T3 built); passing it the freshly re-normalized observation
/// here makes that internal check trivially self-consistent, so the
/// declared-basis equality check below is what actually protects a CLI
/// caller against a tampered `--previous-observation` file (a fully
/// self-consistent forgery would still pass a bare hash-recompute check —
/// design §6.1 is explicit that the hash check alone proves self-consistency,
/// not provenance — which is exactly why this comparison is against the
/// retained previous-capture bytes, not just the declared file's own claim).
pub(in crate::native_cli) fn github_refresh(
    manifest: &Path,
    capture_dir: &Path,
    previous_manifest: &Path,
    previous_capture_dir: &Path,
    previous_observation: Option<&Path>,
) -> Result<NativeCommandResult<Value>, NativeCliError> {
    let capture = normalize_capture(&read_manifest(manifest)?, capture_dir)?;
    let previous_capture =
        normalize_capture(&read_manifest(previous_manifest)?, previous_capture_dir)?;

    if let Some(previous_observation) = previous_observation {
        let declared_basis = read_previous_observation(previous_observation)?;
        if declared_basis != previous_capture.pr_observation {
            return Err(NativeCliError::invalid(format!(
                "{}: declared --previous-observation basis does not match the observation \
                 re-normalized from --previous-manifest/--previous-capture-dir; a declared \
                 basis that disagrees with the retained bytes is an integrity failure, not a \
                 drift",
                previous_observation.display()
            )));
        }
    }

    let refresh_result = classify_refresh(
        &previous_capture.pr_observation,
        &previous_capture.check_evidence,
        &previous_capture.review_findings,
        &capture,
    )
    .map_err(|finding| NativeCliError::Memory(vec![finding]))?;

    // `domain_findings` is the same complete, always-present channel
    // `github_project` exposes — a caller checking only this field (never
    // `refresh_result.disposition` by name) must still see the one
    // condition this command exists to detect. `stale_head` is the sole
    // source here; `refresh_result` still carries the full record-level
    // detail (`disposition`, `review_basis_moved`) alongside it — this adds
    // a command-level channel, it does not relocate anything.
    let mut domain_findings = Vec::new();
    if refresh_result.disposition == RefreshDisposition::StaleHead {
        domain_findings.push(MemoryValidationFinding {
            code: "stale_head".to_owned(),
            location: "$.refresh_result.disposition".to_owned(),
            detail: format!(
                "the observed head {} no longer matches the previous review basis's head {}; \
                 a refresh never rebases — run `github observe` on the new capture instead",
                refresh_result.observed_head_sha, refresh_result.previous_head_sha
            ),
        });
    }
    let domain_finding = !domain_findings.is_empty();
    Ok(NativeCommandResult::with_domain_finding(
        report(
            "casegraphen github refresh",
            json!({
                "refresh_result": refresh_result,
                "domain_findings": domain_findings,
                "accepted": false,
                "mutation_performed": false
            }),
        ),
        domain_finding,
    ))
}

pub(in crate::native_cli) fn github_project(
    manifest: &Path,
    capture_dir: &Path,
    require_independent_review: bool,
) -> Result<NativeCommandResult<Value>, NativeCliError> {
    let capture = normalize_capture(&read_manifest(manifest)?, capture_dir)?;
    let independence = evaluate_independence(
        &capture.pr_observation,
        &capture.review_findings,
        &capture.check_evidence,
        require_independent_review,
    );
    let projection = project_review(
        &capture.pr_observation,
        &capture.check_evidence,
        &capture.review_findings,
        &independence,
        &capture.capture_totals,
        &capture.cross_repository_excluded,
        None,
    );
    // `domain_findings` is the single, complete channel a caller checks for
    // "why did this command report an obstruction" — the same role
    // `memory_check`'s `findings` plays for `valid: false`. It must
    // therefore be everything that can make `domain_finding` true, not just
    // the normalize-time findings: `project_review` already turned an
    // unresolved actionable finding, a failed check, or an unmet
    // `require_independent_review` into a `blocking_findings` entry when it
    // applies (`projection.rs::policy_assignment`), so those are read back
    // here rather than re-deciding any of those questions a second time. A
    // caller that inspects only `result.domain_findings` — never
    // `result.projection.blocking_findings` by name — must still see every
    // reason the command flagged.
    let mut domain_findings = capture.domain_findings.clone();
    domain_findings.extend(projection.blocking_findings.iter().map(|finding| {
        MemoryValidationFinding {
            code: "projection_blocking_finding".to_owned(),
            location: format!("$.projection.blocking_findings[{}]", finding.finding_id),
            detail: finding.reason.clone(),
        }
    }));
    let domain_finding = !domain_findings.is_empty();
    Ok(NativeCommandResult::with_domain_finding(
        report(
            "casegraphen github project",
            json!({
                "projection": projection,
                "independence": independence,
                "domain_findings": domain_findings,
                "accepted": false,
                "mutation_performed": false
            }),
        ),
        domain_finding,
    ))
}

fn normalize_capture(
    manifest: &CaptureManifest,
    capture_dir: &Path,
) -> Result<NormalizedCapture, NativeCliError> {
    normalize(manifest, capture_dir).map_err(|finding| NativeCliError::Memory(vec![finding]))
}

fn read_manifest(path: &Path) -> Result<CaptureManifest, NativeCliError> {
    let manifest: CaptureManifest = parse_strict(read_json(path)?)?;
    if manifest.schema != GITHUB_CAPTURE_MANIFEST_SCHEMA {
        return Err(NativeCliError::invalid(format!(
            "{}: unsupported capture manifest schema {:?}; expected {GITHUB_CAPTURE_MANIFEST_SCHEMA:?}",
            path.display(),
            manifest.schema
        )));
    }
    Ok(manifest)
}

fn read_previous_observation(path: &Path) -> Result<PrObservation, NativeCliError> {
    let observation: PrObservation = parse_strict(read_json(path)?)?;
    if observation.schema != GITHUB_PR_OBSERVATION_SCHEMA {
        return Err(NativeCliError::invalid(format!(
            "{}: unsupported pr_observation schema {:?}; expected {GITHUB_PR_OBSERVATION_SCHEMA:?}",
            path.display(),
            observation.schema
        )));
    }
    Ok(observation)
}
