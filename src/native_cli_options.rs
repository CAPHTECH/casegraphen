use super::{parse_projection_audience, NativeCliError, NativeEvidenceAttachment};
use crate::native_model::ProjectionAudience;
use crate::topology::TopologyReportOptions;
use higher_graphen_core::Id;
use higher_graphen_structure::space::Dimension;
use serde::{Deserialize, Deserializer};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

pub const OPERATION_GATE_PROFILES_SCHEMA: &str = "highergraphen.case.operation_gate_profiles.v1";
const OPERATION_GATE_PROFILES_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResolvedOperationGateOptions {
    pub(super) actor_id: Option<Id>,
    pub(super) capability_ids: Vec<Id>,
    pub(super) operation_scope_id: Option<Id>,
    pub(super) audience: Option<ProjectionAudience>,
    pub(super) source_boundary_id: Option<Id>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct OperationGateInputs {
    actor_id: Option<Id>,
    capability_ids: Option<Vec<Id>>,
    operation_scope_id: Option<Id>,
    audience: Option<ProjectionAudience>,
    source_boundary_id: Option<Id>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MissingOperationGateField {
    Actor,
    Capability,
    OperationScope,
    Audience,
    SourceBoundary,
}

pub(super) enum OperationGateRequirement<'a> {
    Optional,
    Required {
        command: &'a str,
        actor_command: Option<&'a str>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationGateProfilesFile {
    schema: String,
    schema_version: u32,
    profiles: Vec<NamedOperationGateProfile>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedOperationGateProfile {
    name: String,
    #[serde(default, deserialize_with = "deserialize_present_value")]
    actor_id: Option<Id>,
    #[serde(default, deserialize_with = "deserialize_present_value")]
    capability_ids: Option<Vec<Id>>,
    #[serde(default, deserialize_with = "deserialize_present_value")]
    operation_scope_id: Option<Id>,
    #[serde(default, deserialize_with = "deserialize_present_value")]
    audience: Option<ProjectionAudience>,
    #[serde(default, deserialize_with = "deserialize_present_value")]
    source_boundary_id: Option<Id>,
}

fn deserialize_present_value<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl From<NamedOperationGateProfile> for OperationGateInputs {
    fn from(profile: NamedOperationGateProfile) -> Self {
        Self {
            actor_id: profile.actor_id,
            capability_ids: profile.capability_ids,
            operation_scope_id: profile.operation_scope_id,
            audience: profile.audience,
            source_boundary_id: profile.source_boundary_id,
        }
    }
}

#[derive(Default)]
pub(super) struct NativeOptions {
    pub(super) store: Option<PathBuf>,
    pub(super) left_store: Option<PathBuf>,
    pub(super) right_store: Option<PathBuf>,
    pub(super) input: Option<PathBuf>,
    pub(super) policy: Option<PathBuf>,
    pub(super) source_record: Option<PathBuf>,
    pub(super) source_artifact: Option<PathBuf>,
    pub(super) index: Option<PathBuf>,
    pub(super) policy_manifest: Option<PathBuf>,
    pub(super) projection: Option<PathBuf>,
    pub(super) packet: Option<PathBuf>,
    pub(super) manifest: Option<PathBuf>,
    pub(super) capture_dir: Option<PathBuf>,
    pub(super) previous_manifest: Option<PathBuf>,
    pub(super) previous_capture_dir: Option<PathBuf>,
    pub(super) previous_observation: Option<PathBuf>,
    pub(super) output: Option<PathBuf>,
    pub(super) case_space_id: Option<Id>,
    pub(super) left_case_space_id: Option<Id>,
    pub(super) right_case_space_id: Option<Id>,
    pub(super) space_id: Option<Id>,
    pub(super) revision_id: Option<Id>,
    pub(super) base_revision_id: Option<Id>,
    pub(super) plan_id: Option<Id>,
    pub(super) morphism_id: Option<Id>,
    pub(super) target_id: Option<Id>,
    pub(super) cell_id: Option<Id>,
    pub(super) completed_through: Option<Id>,
    pub(super) reviewer_id: Option<Id>,
    pub(super) close_policy_id: Option<Id>,
    pub(super) actor_id: Option<Id>,
    pub(super) gate_actor_id: Option<Id>,
    pub(super) gate_profile: Option<String>,
    pub(super) gate_profile_file: Option<PathBuf>,
    pub(super) retry_step_ids: Vec<Id>,
    pub(super) supersede_trace_ids: Vec<Id>,
    pub(super) evidence_ids: Vec<Id>,
    pub(super) evidence_attachments: Vec<NativeEvidenceAttachment>,
    pub(super) capability_ids: Vec<Id>,
    pub(super) operation_scope_id: Option<Id>,
    pub(super) audience: Option<ProjectionAudience>,
    pub(super) source_boundary_id: Option<Id>,
    pub(super) title: Option<String>,
    pub(super) reason: Option<String>,
    pub(super) lifecycle: Option<String>,
    pub(super) validation_evidence_ids: Vec<Id>,
    pub(super) enabled_worker_kinds: Vec<String>,
    pub(super) run_step: bool,
    pub(super) run_frontier: bool,
    pub(super) max_parallel: Option<usize>,
    pub(super) max_rounds: Option<usize>,
    pub(super) higher_order: bool,
    pub(super) max_dimension: Option<Dimension>,
    pub(super) min_persistence_stages: usize,
    pub(super) adopt_existing_log: bool,
    pub(super) strict: bool,
    pub(super) require_independent_review: bool,
    pub(super) format: NativeOutputFormat,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum NativeOutputFormat {
    #[default]
    Json,
    Text,
}

impl NativeOptions {
    /// `segment` names the command being parsed, purely for the
    /// unknown-argument refusal below. Every real call site passes only
    /// the top-level group (`"space"`, `"cell"`, `"morphism"`, ...), not
    /// the full two-word command — so `space inspect --strict` refuses
    /// with "...for space", not "...for space inspect", and `space reason
    /// --strict` (which does accept it) is not distinguished from the
    /// sibling operations that do not. That coarser granularity is
    /// intentional, not an oversight: it is not a flag registry — inventing
    /// one to enumerate *valid* flags per command, or even per-operation
    /// labels, would be new code the minimalism rule rejects. Naming the
    /// group being parsed is the cheap half that is actually useful.
    pub(super) fn parse(
        segment: &'static str,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        Self::parse_internal(segment, args, false, false)
    }

    pub(super) fn parse_with_strict(
        segment: &'static str,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        Self::parse_internal(segment, args, true, false)
    }

    pub(super) fn parse_reason(
        segment: &'static str,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        Self::parse_internal(segment, args, true, true)
    }

    /// `--format text` without `--strict`: `space history` accepts a
    /// rendered view of the log (native_cli_text.rs's fold) but has no
    /// `strict` field to carry that flag to, so accepting it here would
    /// parse successfully and then silently drop it.
    pub(super) fn parse_text_only(
        segment: &'static str,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        Self::parse_internal(segment, args, false, true)
    }

    fn parse_internal(
        segment: &'static str,
        args: impl IntoIterator<Item = OsString>,
        strict_allowed: bool,
        text_allowed: bool,
    ) -> Result<Self, NativeCliError> {
        let mut options = Self::default();
        let mut format_seen = false;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            options.consume_arg(
                segment,
                &arg,
                &mut args,
                &mut format_seen,
                strict_allowed,
                text_allowed,
            )?;
        }
        if !format_seen {
            return Err(NativeCliError::usage("--format json is required"));
        }
        Ok(options)
    }

    fn consume_arg(
        &mut self,
        segment: &'static str,
        arg: &OsString,
        args: &mut impl Iterator<Item = OsString>,
        format_seen: &mut bool,
        strict_allowed: bool,
        text_allowed: bool,
    ) -> Result<(), NativeCliError> {
        match arg.to_str() {
            Some("--format") => {
                self.format = require_format(args, text_allowed)?;
                *format_seen = true;
            }
            Some("--store") => self.store = Some(require_path(args, "--store")?),
            Some("--left-store") => self.left_store = Some(require_path(args, "--left-store")?),
            Some("--right-store") => self.right_store = Some(require_path(args, "--right-store")?),
            Some("--input") => {
                let input = require_path(args, "--input")?;
                self.input = Some(input.clone());
                self.evidence_attachments.push(NativeEvidenceAttachment {
                    input,
                    satisfies_ids: Vec::new(),
                    artifact_paths: Vec::new(),
                });
            }
            Some("--policy-manifest") => {
                self.policy_manifest = Some(require_path(args, "--policy-manifest")?)
            }
            Some("--policy") => self.policy = Some(require_path(args, "--policy")?),
            Some("--source-record") => {
                self.source_record = Some(require_path(args, "--source-record")?)
            }
            Some("--source-artifact") => {
                self.source_artifact = Some(require_path(args, "--source-artifact")?)
            }
            Some("--index") => self.index = Some(require_path(args, "--index")?),
            Some("--projection") => self.projection = Some(require_path(args, "--projection")?),
            Some("--packet") => self.packet = Some(require_path(args, "--packet")?),
            Some("--manifest") => self.manifest = Some(require_path(args, "--manifest")?),
            Some("--capture-dir") => self.capture_dir = Some(require_path(args, "--capture-dir")?),
            Some("--previous-manifest") => {
                self.previous_manifest = Some(require_path(args, "--previous-manifest")?)
            }
            Some("--previous-capture-dir") => {
                self.previous_capture_dir = Some(require_path(args, "--previous-capture-dir")?)
            }
            Some("--previous-observation") => {
                self.previous_observation = Some(require_path(args, "--previous-observation")?)
            }
            Some("--output") => self.output = Some(require_path(args, "--output")?),
            Some("--case-space-id") => {
                self.case_space_id = Some(require_id(args, "--case-space-id")?)
            }
            Some("--left-case-space-id") => {
                self.left_case_space_id = Some(require_id(args, "--left-case-space-id")?)
            }
            Some("--right-case-space-id") => {
                self.right_case_space_id = Some(require_id(args, "--right-case-space-id")?)
            }
            Some("--space-id") => self.space_id = Some(require_id(args, "--space-id")?),
            Some("--revision-id") => self.revision_id = Some(require_id(args, "--revision-id")?),
            Some("--base-revision") | Some("--base-revision-id") => {
                self.base_revision_id = Some(require_id(args, "--base-revision-id")?)
            }
            Some("--plan-id") => self.plan_id = Some(require_id(args, "--plan-id")?),
            Some("--morphism-id") => self.morphism_id = Some(require_id(args, "--morphism-id")?),
            Some("--target-id") => self.target_id = Some(require_id(args, "--target-id")?),
            Some("--cell-id") => self.cell_id = Some(require_id(args, "--cell-id")?),
            Some("--completed-through") => {
                self.completed_through = Some(require_id(args, "--completed-through")?)
            }
            Some("--reviewer-id") => self.reviewer_id = Some(require_id(args, "--reviewer-id")?),
            Some("--close-policy-id") => {
                self.close_policy_id = Some(require_id(args, "--close-policy-id")?)
            }
            Some("--actor-id") => self.actor_id = Some(require_id(args, "--actor-id")?),
            Some("--gate-actor-id") => {
                self.gate_actor_id = Some(require_id(args, "--gate-actor-id")?)
            }
            Some("--gate-profile") => {
                self.gate_profile = Some(require_string(args, "--gate-profile")?)
            }
            Some("--gate-profile-file") => {
                self.gate_profile_file = Some(require_path(args, "--gate-profile-file")?)
            }
            Some("--retry-step") => self.retry_step_ids.push(require_id(args, "--retry-step")?),
            Some("--supersede-trace") => self
                .supersede_trace_ids
                .push(require_id(args, "--supersede-trace")?),
            Some("--evidence-id") => self.evidence_ids.push(require_id(args, "--evidence-id")?),
            Some("--satisfies") => self
                .evidence_attachments
                .last_mut()
                .ok_or_else(|| {
                    NativeCliError::usage("--satisfies must follow the --input it belongs to")
                })?
                .satisfies_ids
                .push(require_id(args, "--satisfies")?),
            Some("--artifact") => {
                let artifact = require_path(args, "--artifact")?;
                self.evidence_attachments
                    .last_mut()
                    .ok_or_else(|| {
                        NativeCliError::usage("--artifact must follow the --input it belongs to")
                    })?
                    .artifact_paths
                    .push(artifact);
            }
            Some("--capability-id") => self
                .capability_ids
                .push(require_id(args, "--capability-id")?),
            Some("--operation-scope-id") => {
                self.operation_scope_id = Some(require_id(args, "--operation-scope-id")?)
            }
            Some("--audience") => {
                self.audience = Some(parse_projection_audience(&require_string(
                    args,
                    "--audience",
                )?)?)
            }
            Some("--source-boundary-id") => {
                self.source_boundary_id = Some(require_id(args, "--source-boundary-id")?)
            }
            Some("--title") => self.title = Some(require_string(args, "--title")?),
            Some("--reason") => self.reason = Some(require_string(args, "--reason")?),
            Some("--to") => self.lifecycle = Some(require_string(args, "--to")?),
            Some("--validation-evidence-id") => self
                .validation_evidence_ids
                .push(require_id(args, "--validation-evidence-id")?),
            Some("--enable-worker") => self
                .enabled_worker_kinds
                .push(require_string(args, "--enable-worker")?),
            Some("--step") => self.run_step = true,
            Some("--frontier") => self.run_frontier = true,
            Some("--max-parallel") => {
                self.max_parallel = Some(require_usize(args, "--max-parallel")?)
            }
            Some("--max-rounds") => self.max_rounds = Some(require_usize(args, "--max-rounds")?),
            Some("--higher-order") => self.higher_order = true,
            Some("--max-dimension") => {
                self.max_dimension = Some(require_dimension(args, "--max-dimension")?)
            }
            Some("--min-persistence") | Some("--min-persistence-stages") => {
                self.min_persistence_stages = require_usize(args, "--min-persistence")?;
            }
            Some("--adopt-existing-log") => self.adopt_existing_log = true,
            Some("--strict") if strict_allowed => self.strict = true,
            Some("--require-independent-review") => self.require_independent_review = true,
            Some(_) | None => {
                return Err(NativeCliError::usage(format!(
                    "unsupported native argument {arg:?} for {segment}"
                )))
            }
        }
        Ok(())
    }

    pub(super) fn topology_options(&self) -> TopologyReportOptions {
        if self.higher_order {
            TopologyReportOptions::higher_order(self.max_dimension, self.min_persistence_stages)
        } else {
            TopologyReportOptions::baseline()
        }
    }

    pub(super) fn resolve_operation_gate_options(
        &self,
        requirement: OperationGateRequirement<'_>,
    ) -> Result<ResolvedOperationGateOptions, NativeCliError> {
        let profile = self.selected_operation_gate_profile()?;
        let flags = OperationGateInputs {
            actor_id: self.actor_id.clone(),
            capability_ids: (!self.capability_ids.is_empty()).then(|| self.capability_ids.clone()),
            operation_scope_id: self.operation_scope_id.clone(),
            audience: self.audience,
            source_boundary_id: self.source_boundary_id.clone(),
        };
        let resolved = resolve_operation_gate_inputs(flags, profile);
        if let OperationGateRequirement::Required {
            command,
            actor_command,
        } = requirement
        {
            require_complete_operation_gate_inputs(&resolved).map_err(|missing| match missing {
                MissingOperationGateField::Actor => NativeCliError::usage(match actor_command {
                    Some(actor_command) => {
                        format!("--actor-id <id> is required for {actor_command}")
                    }
                    None => "--actor-id <id> is required".to_owned(),
                }),
                // A pre-flight completeness check, not a gate decision: the
                // flag is missing and nothing was evaluated, so this is
                // `usage` (fix the call), not `gate_violation` — and the
                // message says plainly what is missing instead of wearing
                // `check_operation_gate`'s "violates: ..." phrasing, which
                // is how a caller (and the SKILL.md guidance) could mistake
                // the single most common gate mistake for something it
                // is not.
                MissingOperationGateField::Capability => {
                    NativeCliError::usage(format!("--capability-id <id> is required for {command}"))
                }
                MissingOperationGateField::OperationScope => NativeCliError::usage(format!(
                    "--operation-scope-id <id> is required for {command}"
                )),
                MissingOperationGateField::Audience => NativeCliError::usage(format!(
                    "--audience audit|system is required for {command}"
                )),
                MissingOperationGateField::SourceBoundary => NativeCliError::usage(format!(
                    "--source-boundary-id <id> is required for {command}"
                )),
            })?;
        }
        Ok(ResolvedOperationGateOptions {
            actor_id: resolved.actor_id,
            capability_ids: resolved.capability_ids.unwrap_or_default(),
            operation_scope_id: resolved.operation_scope_id,
            audience: resolved.audience,
            source_boundary_id: resolved.source_boundary_id,
        })
    }

    fn selected_operation_gate_profile(&self) -> Result<OperationGateInputs, NativeCliError> {
        let (name, path) = match (&self.gate_profile, &self.gate_profile_file) {
            (None, None) => return Ok(OperationGateInputs::default()),
            (Some(_), None) => {
                return Err(NativeCliError::usage(
                    "--gate-profile-file <path> is required with --gate-profile <name>",
                ))
            }
            (None, Some(_)) => {
                return Err(NativeCliError::usage(
                    "--gate-profile <name> is required with --gate-profile-file <path>",
                ))
            }
            (Some(name), Some(path)) => (name, path),
        };
        if name.trim().is_empty() {
            return Err(NativeCliError::invalid(
                "operation gate profile name must not be empty",
            ));
        }
        let raw = fs::read_to_string(path).map_err(|source| NativeCliError::Io {
            path: path.clone(),
            source,
        })?;
        let profile_file: OperationGateProfilesFile = serde_json::from_str(&raw)?;
        validate_operation_gate_profile_file(&profile_file)?;
        profile_file
            .profiles
            .into_iter()
            .find(|profile| profile.name == *name)
            .map(OperationGateInputs::from)
            .ok_or_else(|| {
                NativeCliError::invalid(format!(
                    "operation gate profile {name:?} was not found in {}",
                    path.display()
                ))
            })
    }

    pub(super) fn require_store(&self) -> Result<PathBuf, NativeCliError> {
        self.store
            .clone()
            .ok_or_else(|| NativeCliError::usage("--store <dir> is required"))
    }

    pub(super) fn require_path(&self, flag: &str) -> Result<PathBuf, NativeCliError> {
        match flag {
            "--input" => self.input.clone(),
            "--policy" => self.policy.clone(),
            "--source-record" => self.source_record.clone(),
            "--source-artifact" => self.source_artifact.clone(),
            "--index" => self.index.clone(),
            "--projection" => self.projection.clone(),
            "--packet" => self.packet.clone(),
            "--left-store" => self.left_store.clone(),
            "--right-store" => self.right_store.clone(),
            "--manifest" => self.manifest.clone(),
            "--capture-dir" => self.capture_dir.clone(),
            "--previous-manifest" => self.previous_manifest.clone(),
            "--previous-capture-dir" => self.previous_capture_dir.clone(),
            "--previous-observation" => self.previous_observation.clone(),
            _ => None,
        }
        .ok_or_else(|| NativeCliError::usage(format!("{flag} <path> is required")))
    }

    pub(super) fn require_id(&self, flag: &str) -> Result<Id, NativeCliError> {
        match flag {
            "--case-space-id" => self.case_space_id.clone(),
            "--left-case-space-id" => self.left_case_space_id.clone(),
            "--right-case-space-id" => self.right_case_space_id.clone(),
            "--space-id" => self.space_id.clone(),
            "--revision-id" => self.revision_id.clone(),
            "--plan-id" => self.plan_id.clone(),
            "--reviewer-id" => self.reviewer_id.clone(),
            "--morphism-id" => self.morphism_id.clone(),
            "--target-id" => self.target_id.clone(),
            "--cell-id" => self.cell_id.clone(),
            "--completed-through" => self.completed_through.clone(),
            "--actor-id" => self.actor_id.clone(),
            _ => None,
        }
        .ok_or_else(|| NativeCliError::usage(format!("{flag} <id> is required")))
    }

    pub(super) fn require_string(&self, flag: &str) -> Result<String, NativeCliError> {
        match flag {
            "--title" => self.title.clone(),
            "--reason" => self.reason.clone(),
            "--to" => self.lifecycle.clone(),
            _ => None,
        }
        .ok_or_else(|| NativeCliError::usage(format!("{flag} <text> is required")))
    }
}

fn resolve_operation_gate_inputs(
    flags: OperationGateInputs,
    profile: OperationGateInputs,
) -> OperationGateInputs {
    OperationGateInputs {
        actor_id: flags.actor_id.or(profile.actor_id),
        capability_ids: flags.capability_ids.or(profile.capability_ids),
        operation_scope_id: flags.operation_scope_id.or(profile.operation_scope_id),
        audience: flags.audience.or(profile.audience),
        source_boundary_id: flags.source_boundary_id.or(profile.source_boundary_id),
    }
}

fn require_complete_operation_gate_inputs(
    resolved: &OperationGateInputs,
) -> Result<(), MissingOperationGateField> {
    if resolved.actor_id.is_none() {
        return Err(MissingOperationGateField::Actor);
    }
    if resolved.operation_scope_id.is_none() {
        return Err(MissingOperationGateField::OperationScope);
    }
    if resolved.audience.is_none() {
        return Err(MissingOperationGateField::Audience);
    }
    if resolved.source_boundary_id.is_none() {
        return Err(MissingOperationGateField::SourceBoundary);
    }
    if resolved.capability_ids.is_none() {
        return Err(MissingOperationGateField::Capability);
    }
    Ok(())
}

fn validate_operation_gate_profile_file(
    profile_file: &OperationGateProfilesFile,
) -> Result<(), NativeCliError> {
    if profile_file.schema != OPERATION_GATE_PROFILES_SCHEMA {
        return Err(NativeCliError::invalid(format!(
            "unsupported operation gate profiles schema {:?}; expected {OPERATION_GATE_PROFILES_SCHEMA:?}",
            profile_file.schema
        )));
    }
    if profile_file.schema_version != OPERATION_GATE_PROFILES_SCHEMA_VERSION {
        return Err(NativeCliError::invalid(format!(
            "unsupported operation gate profiles schema version {}; expected {OPERATION_GATE_PROFILES_SCHEMA_VERSION}",
            profile_file.schema_version
        )));
    }
    if profile_file.profiles.is_empty() {
        return Err(NativeCliError::invalid(
            "operation gate profiles file must contain at least one profile",
        ));
    }
    let mut names = BTreeSet::new();
    for profile in &profile_file.profiles {
        if profile.name.trim().is_empty() {
            return Err(NativeCliError::invalid(
                "operation gate profile name must not be empty",
            ));
        }
        if !names.insert(profile.name.as_str()) {
            return Err(NativeCliError::invalid(format!(
                "duplicate operation gate profile name {:?}",
                profile.name
            )));
        }
        if profile.capability_ids.as_ref().is_some_and(Vec::is_empty) {
            return Err(NativeCliError::invalid(format!(
                "operation gate profile {:?} capability_ids must not be empty when present",
                profile.name
            )));
        }
    }
    Ok(())
}

pub(super) fn required_segment(
    args: &mut impl Iterator<Item = OsString>,
    label: &str,
) -> Result<OsString, NativeCliError> {
    args.next()
        .ok_or_else(|| NativeCliError::usage(format!("{label} is required")))
}

/// The single recognizer for what a `--format` value token means. Both the
/// strict per-command parser below (which requires the flag be present and
/// rejects `text` unless the command allows it) and `scan_requested_format`
/// (a lenient pre-scan of raw argv, used to render a refusal from a parse
/// that may have failed before ever reaching `--format`) resolve a token
/// through this one match. A second recognizer answering "is this token a
/// format" differently from this one is exactly the one-rule-two-places
/// shape this crate forbids.
fn recognized_format(value: &str) -> Option<NativeOutputFormat> {
    match value {
        "json" => Some(NativeOutputFormat::Json),
        "text" => Some(NativeOutputFormat::Text),
        _ => None,
    }
}

fn require_format(
    args: &mut impl Iterator<Item = OsString>,
    text_allowed: bool,
) -> Result<NativeOutputFormat, NativeCliError> {
    match required_segment(args, "--format value")?
        .to_str()
        .and_then(recognized_format)
    {
        Some(NativeOutputFormat::Text) if !text_allowed => {
            Err(NativeCliError::usage("--format json is required"))
        }
        Some(format) => Ok(format),
        None => Err(NativeCliError::usage("--format json is required")),
    }
}

/// Resolves the `--format` a refusal should render as, from the same raw
/// argv `Command::parse` was given, for the one case where that is the only
/// information available: `Command::parse` itself failed, so there is no
/// `Command` to ask (`main_entry::parse_and_run` never reaches this scanner
/// once parsing has succeeded — the resolved `command.format()` is
/// authoritative there instead).
///
/// This does **not** match `parse_internal`'s left-to-right *consumption* —
/// it matches token *order*, which is a different thing. `parse_internal`
/// knows which flags take a following token as a value and skips over
/// those; this scan does not, because it cannot: it exists precisely for
/// the case where parsing never got far enough to know that. A value-typed
/// flag (`--reason`, `--title`, ...) whose value is literally the string
/// `--format` will make this scan see two `--format` tokens where the real
/// parser — had it succeeded — would have seen only the second as the
/// actual flag. Teaching this scanner that distinction would mean copying
/// `consume_arg`'s knowledge of every flag's arity, which is worse than the
/// residual: in the parse-failure domain there is no authoritative answer
/// to disagree with regardless, so **first** recognized `--format <value>`
/// wins, not last. First-wins fails toward the machine-readable form a
/// caller most likely wants: a spurious later `--format text` sitting in a
/// value position cannot silently override a real, earlier `--format
/// json`.
///
/// An argv with **no** `--format` token anywhere defaults to `Text`, not
/// `NativeOutputFormat::default()` (json) — deliberately not the same
/// default a successful command uses. `--format json` is required for
/// every successful command, so "no `--format` anywhere in this argv" only
/// happens for an invocation malformed enough that it never got there: a
/// bare `casegraphen`, a missing sub-command, a segment typo. Those are
/// exactly the callers who gave no signal they wanted machine-readable
/// output at all, and a plain-prose usage refusal (which is what `casegraphen`
/// printed before this scanner existed) serves them better than a
/// single-line JSON object that quietly drops the appended usage block
/// `CliError::Usage`'s `Display` carries.
pub(crate) fn scan_requested_format(args: &[OsString]) -> NativeOutputFormat {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg.to_str() == Some("--format") {
            if let Some(format) = iter
                .next()
                .and_then(|value| value.to_str())
                .and_then(recognized_format)
            {
                return format;
            }
        }
    }
    NativeOutputFormat::Text
}

pub(super) fn require_path(
    args: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> Result<PathBuf, NativeCliError> {
    let value = required_segment(args, flag)?;
    let path = PathBuf::from(value);
    reject_unsafe_path(flag, &path)?;
    Ok(path)
}

pub(super) fn require_string(
    args: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> Result<String, NativeCliError> {
    required_segment(args, flag)?
        .into_string()
        .map_err(|_| NativeCliError::usage(format!("{flag} must be UTF-8")))
}

pub(super) fn require_id(
    args: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> Result<Id, NativeCliError> {
    Ok(Id::new(require_string(args, flag)?)?)
}

fn require_dimension(
    args: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> Result<Dimension, NativeCliError> {
    require_string(args, flag)?
        .parse::<Dimension>()
        .map_err(|_| NativeCliError::usage(format!("invalid integer for {flag}")))
}

fn require_usize(
    args: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> Result<usize, NativeCliError> {
    require_string(args, flag)?
        .parse::<usize>()
        .map_err(|_| NativeCliError::usage(format!("invalid integer for {flag}")))
}

fn reject_unsafe_path(flag: &str, path: &Path) -> Result<(), NativeCliError> {
    if path.as_os_str().is_empty() {
        return Err(NativeCliError::usage(format!("{flag} must not be empty")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbtest::arbitrary::Arbitrary;

    #[test]
    fn operation_gate_resolution_is_per_field_flag_first_and_complete_or_refused() {
        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let present = <[bool; 10]>::arbitrary(u)?;
                let flags = OperationGateInputs {
                    actor_id: present[0].then(|| id("actor:flag")),
                    capability_ids: present[2].then(|| vec![id("capability:flag")]),
                    operation_scope_id: present[4].then(|| id("case_space:flag")),
                    audience: present[6].then_some(ProjectionAudience::Audit),
                    source_boundary_id: present[8].then(|| id("source_boundary:flag")),
                };
                let profile = OperationGateInputs {
                    actor_id: present[1].then(|| id("actor:profile")),
                    capability_ids: present[3].then(|| vec![id("capability:profile")]),
                    operation_scope_id: present[5].then(|| id("case_space:profile")),
                    audience: present[7].then_some(ProjectionAudience::System),
                    source_boundary_id: present[9].then(|| id("source_boundary:profile")),
                };

                let resolved = resolve_operation_gate_inputs(flags.clone(), profile.clone());

                assert_eq!(
                    resolved.actor_id,
                    flags.actor_id.clone().or(profile.actor_id)
                );
                assert_eq!(
                    resolved.capability_ids,
                    flags.capability_ids.clone().or(profile.capability_ids)
                );
                assert_eq!(
                    resolved.operation_scope_id,
                    flags
                        .operation_scope_id
                        .clone()
                        .or(profile.operation_scope_id)
                );
                assert_eq!(resolved.audience, flags.audience.or(profile.audience));
                assert_eq!(
                    resolved.source_boundary_id,
                    flags.source_boundary_id.or(profile.source_boundary_id)
                );

                let command_accepts = require_complete_operation_gate_inputs(&resolved).is_ok();
                let every_field_was_supplied = present
                    .chunks_exact(2)
                    .all(|field_sources| field_sources[0] || field_sources[1]);
                assert_eq!(command_accepts, every_field_was_supplied);
                Ok(())
            },
        );
    }

    fn id(value: &str) -> Id {
        Id::new(value.to_owned()).expect("test id")
    }
}
