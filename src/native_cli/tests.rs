use super::*;
use std::ffi::OsString;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn strict_is_limited_to_finding_carrying_reports() {
    for command in [
        NativeCliCommand::parse(
            "space",
            args(&[
                "reason",
                "--store",
                "store",
                "--case-space-id",
                "case_space:demo",
                "--strict",
                "--format",
                "json",
            ]),
        ),
        NativeCliCommand::parse(
            "obstruction",
            args(&[
                "list",
                "--store",
                "store",
                "--case-space-id",
                "case_space:demo",
                "--strict",
                "--format",
                "json",
            ]),
        ),
        NativeCliCommand::parse(
            "invariant",
            args(&[
                "check",
                "--store",
                "store",
                "--case-space-id",
                "case_space:demo",
                "--strict",
                "--format",
                "json",
            ]),
        ),
        NativeCliCommand::parse(
            "invariant",
            args(&[
                "close-check",
                "--store",
                "store",
                "--case-space-id",
                "case_space:demo",
                "--base-revision-id",
                "revision:demo",
                "--strict",
                "--format",
                "json",
            ]),
        ),
    ] {
        assert!(command.expect("strict report command").strict());
    }

    for mode in ["--step", "--frontier"] {
        let command = NativeCliCommand::parse(
            "run",
            args(&[
                mode,
                "--store",
                "store",
                "--case-space-id",
                "case_space:demo",
                "--plan-id",
                "plan:demo",
                "--base-revision-id",
                "revision:demo",
                "--actor-id",
                "actor:demo",
                "--capability-id",
                "capability:dispatch",
                "--operation-scope-id",
                "case_space:demo",
                "--audience",
                "audit",
                "--source-boundary-id",
                "source_boundary:demo",
                "--strict",
                "--format",
                "json",
            ]),
        )
        .expect("strict run command");
        assert!(command.strict());
    }

    for (namespace, operation) in [
        ("space", "frontier"),
        ("space", "evidence"),
        ("space", "inspect"),
        ("completion", "candidates"),
    ] {
        let error = NativeCliCommand::parse(
            namespace,
            args(&[
                operation,
                "--store",
                "store",
                "--case-space-id",
                "case_space:demo",
                "--strict",
                "--format",
                "json",
            ]),
        )
        .expect_err("strict must not apply to a report without domain findings");
        assert!(matches!(
            error,
            NativeCliError::Usage(message) if message.contains("unsupported native argument")
        ));
    }
}

#[test]
fn parses_space_commands_as_canonical_native_surface() {
    let command = NativeCliCommand::parse(
        "space",
        args(&[
            "reason",
            "--store",
            "store",
            "--case-space-id",
            "case_space:demo",
            "--format",
            "json",
        ]),
    )
    .expect("space reason command");

    assert!(matches!(
        command,
        NativeCliCommand::CaseReason {
            section: NativeReasonSection::Reason,
            ..
        }
    ));

    let topology = NativeCliCommand::parse(
        "space",
        args(&[
            "topology",
            "diff",
            "--left-store",
            "left",
            "--left-case-space-id",
            "case_space:left",
            "--right-store",
            "right",
            "--right-case-space-id",
            "case_space:right",
            "--format",
            "json",
        ]),
    )
    .expect("space topology diff command");

    assert!(matches!(
        topology,
        NativeCliCommand::CaseTopologyDiff { .. }
    ));
}

#[test]
fn parses_value_namespaces_to_existing_native_operations() {
    assert_value_namespace_reason("obstruction", "list", NativeReasonSection::Obstructions);
    assert_value_namespace_reason("completion", "candidates", NativeReasonSection::Completions);
    assert_projection_namespace();
    assert_invariant_namespace();
    assert_equivalence_namespace();
}

fn assert_value_namespace_reason(namespace: &str, operation: &str, section: NativeReasonSection) {
    let command = NativeCliCommand::parse(
        namespace,
        args(&[
            operation,
            "--store",
            "store",
            "--case-space-id",
            "case_space:demo",
            "--format",
            "json",
        ]),
    )
    .expect("value namespace command");
    assert!(matches!(
        command,
        NativeCliCommand::CaseReason {
            section: parsed,
            ..
        } if parsed == section
    ));
}

fn assert_projection_namespace() {
    let projection = NativeCliCommand::parse(
        "projection",
        args(&[
            "apply",
            "--store",
            "store",
            "--case-space-id",
            "case_space:demo",
            "--projection",
            "projection.json",
            "--format",
            "json",
        ]),
    )
    .expect("projection apply command");
    assert!(matches!(
        projection,
        NativeCliCommand::ProjectionApply { .. }
    ));
}

fn assert_invariant_namespace() {
    let invariant = NativeCliCommand::parse(
        "invariant",
        args(&[
            "check",
            "--store",
            "store",
            "--case-space-id",
            "case_space:demo",
            "--format",
            "json",
        ]),
    )
    .expect("invariant check command");
    assert!(matches!(invariant, NativeCliCommand::InvariantCheck { .. }));
}

fn assert_equivalence_namespace() {
    let equivalence = NativeCliCommand::parse(
        "equivalence",
        args(&[
            "check",
            "--left-store",
            "left",
            "--left-case-space-id",
            "case_space:left",
            "--right-store",
            "right",
            "--right-case-space-id",
            "case_space:right",
            "--format",
            "json",
        ]),
    )
    .expect("equivalence check command");
    assert!(matches!(
        equivalence,
        NativeCliCommand::EquivalenceCheck { .. }
    ));
}

#[test]
fn parses_lift_adapters() {
    let workflow = NativeCliCommand::parse(
        "lift",
        args(&[
            "workflow",
            "--store",
            "store",
            "--input",
            "workflow.graph.json",
            "--revision-id",
            "revision:lifted",
            "--format",
            "json",
        ]),
    )
    .expect("workflow lift command");

    assert!(matches!(
        workflow,
        NativeCliCommand::LiftStructuredSource { adapter, .. } if adapter == "workflow"
    ));

    let github_issues = NativeCliCommand::parse(
        "lift",
        args(&[
            "github-issues",
            "--store",
            "store",
            "--input",
            "github.issue-snapshot.json",
            "--revision-id",
            "revision:lifted",
            "--format",
            "json",
        ]),
    )
    .expect("GitHub issues lift command");
    assert!(matches!(
        github_issues,
        NativeCliCommand::LiftStructuredSource { adapter, .. } if adapter == "github-issues"
    ));

    let native = NativeCliCommand::parse(
        "lift",
        args(&[
            "native",
            "--store",
            "store",
            "--input",
            "native.case.space.json",
            "--revision-id",
            "revision:lifted",
            "--format",
            "json",
        ]),
    )
    .expect("native lift command");
    assert!(matches!(native, NativeCliCommand::CaseImport { .. }));
}

#[test]
fn parses_native_mutation_command_families() {
    let plan = NativeCliCommand::parse(
        "plan",
        args(&[
            "accept",
            "--store",
            "store",
            "--case-space-id",
            "case_space:demo",
            "--plan-id",
            "plan:demo",
            "--reviewer-id",
            "reviewer:demo",
            "--reason",
            "Accepted execution plan",
            "--base-revision-id",
            "revision:base",
            "--actor-id",
            "actor:demo",
            "--capability-id",
            "capability:plan-review",
            "--operation-scope-id",
            "case_space:demo",
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:demo",
            "--format",
            "json",
        ]),
    )
    .expect("plan accept command");
    assert!(matches!(
        plan,
        NativeCliCommand::PlanReview {
            action: ReviewAction::Accept,
            gate_options,
            ..
        } if gate_options.capability_ids
            == vec![Id::new("capability:plan-review").expect("capability id")]
    ));

    let review = NativeCliCommand::parse(
        "review",
        args(&[
            "waive",
            "--store",
            "store",
            "--case-space-id",
            "case_space:demo",
            "--target-id",
            "work:demo",
            "--reviewer-id",
            "reviewer:demo",
            "--reason",
            "Deferred by waiver",
            "--base-revision-id",
            "revision:base",
            "--evidence-id",
            "evidence:demo",
            "--actor-id",
            "actor:demo",
            "--capability-id",
            "capability:review",
            "--operation-scope-id",
            "case_space:demo",
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:demo",
            "--format",
            "json",
        ]),
    )
    .expect("review waive command");
    assert!(matches!(
        review,
        NativeCliCommand::Review {
            action: ReviewAction::Defer,
            evidence_ids,
            ..
        } if evidence_ids == vec![Id::new("evidence:demo").expect("evidence id")]
    ));

    let evidence = NativeCliCommand::parse(
        "evidence",
        args(&[
            "attach",
            "--store",
            "store",
            "--case-space-id",
            "case_space:demo",
            "--base-revision-id",
            "revision:base",
            "--input",
            "evidence.json",
            "--satisfies",
            "goal:demo",
            "--satisfies",
            "case:demo",
            "--artifact",
            "build.log",
            "--artifact",
            "results.xcresult",
            "--input",
            "second-evidence.json",
            "--satisfies",
            "work:demo",
            "--actor-id",
            "actor:demo",
            "--capability-id",
            "capability:evidence-attach",
            "--operation-scope-id",
            "case_space:demo",
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:demo",
            "--format",
            "json",
        ]),
    )
    .expect("evidence attach command");
    assert!(matches!(
        evidence,
        NativeCliCommand::EvidenceAttach { attachments, .. }
            if attachments == vec![
                NativeEvidenceAttachment {
                    input: PathBuf::from("evidence.json"),
                    satisfies_ids: vec![
                        Id::new("goal:demo").expect("goal id"),
                        Id::new("case:demo").expect("case id"),
                    ],
                    artifact_paths: vec![
                        PathBuf::from("build.log"),
                        PathBuf::from("results.xcresult"),
                    ],
                },
                NativeEvidenceAttachment {
                    input: PathBuf::from("second-evidence.json"),
                    satisfies_ids: vec![Id::new("work:demo").expect("work id")],
                    artifact_paths: Vec::new(),
                },
            ]
    ));

    let transition = NativeCliCommand::parse(
        "cell",
        args(&[
            "transition",
            "--store",
            "store",
            "--case-space-id",
            "case_space:demo",
            "--base-revision-id",
            "revision:base",
            "--cell-id",
            "work:demo",
            "--to",
            "resolved",
            "--actor-id",
            "actor:demo",
            "--capability-id",
            "capability:cell-transition",
            "--operation-scope-id",
            "case_space:demo",
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:demo",
            "--format",
            "json",
        ]),
    )
    .expect("cell transition command");
    assert!(matches!(
        transition,
        NativeCliCommand::CellTransition { lifecycle, .. } if lifecycle == "resolved"
    ));
}

#[test]
fn run_gate_actor_alias_must_equal_the_log_actor() {
    let error = NativeCliCommand::parse(
        "run",
        args(&[
            "--step",
            "--store",
            "store",
            "--case-space-id",
            "case_space:demo",
            "--plan-id",
            "plan:demo",
            "--base-revision-id",
            "revision:demo",
            "--actor-id",
            "actor:log",
            "--gate-actor-id",
            "actor:gate",
            "--capability-id",
            "capability:dispatch",
            "--operation-scope-id",
            "case_space:demo",
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:demo",
            "--format",
            "json",
        ]),
    )
    .expect_err("split actor identity must be rejected");

    assert!(matches!(
        error,
        NativeCliError::Usage(message)
            if message.contains("must equal --actor-id")
    ));
}

/// One instance per `NativeCliError` shape the `error_code`/`refusal_data`
/// match distinguishes, including every `NativeStoreError` sub-variant.
/// `Worker(_)` is not built here: `exec::worker::WorkerError` has no public
/// constructor outside its own module, so it cannot be constructed from
/// this test — its single code is still guaranteed reachable because
/// `error_code`'s match has no wildcard arm, so the compiler itself refuses
/// to build if a variant (including `Worker`) is ever left unhandled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErrorShape {
    Usage,
    UnsupportedArgumentValue,
    Invalid,
    StaleRevision,
    StalePlanRevision,
    GateViolation,
    StoreIntegrity,
    Core,
    StoreIo,
    StoreJson,
    StoreUnsupportedSchema,
    StoreUnsupportedVersion,
    StoreMissingCase,
    StoreExistingCase,
    StoreLockUnavailable,
    StoreReplayMismatch,
    StoreInvalidMorphism,
    Review,
    Eval,
    Io,
    Json,
}

const ALL_ERROR_SHAPES: &[ErrorShape] = &[
    ErrorShape::Usage,
    ErrorShape::UnsupportedArgumentValue,
    ErrorShape::Invalid,
    ErrorShape::StaleRevision,
    ErrorShape::StalePlanRevision,
    ErrorShape::GateViolation,
    ErrorShape::StoreIntegrity,
    ErrorShape::Core,
    ErrorShape::StoreIo,
    ErrorShape::StoreJson,
    ErrorShape::StoreUnsupportedSchema,
    ErrorShape::StoreUnsupportedVersion,
    ErrorShape::StoreMissingCase,
    ErrorShape::StoreExistingCase,
    ErrorShape::StoreLockUnavailable,
    ErrorShape::StoreReplayMismatch,
    ErrorShape::StoreInvalidMorphism,
    ErrorShape::Review,
    ErrorShape::Eval,
    ErrorShape::Io,
    ErrorShape::Json,
];

/// Builds one `NativeCliError` of the given shape, with `seed` threaded
/// into whatever free-text field the shape carries (and nowhere else — ids
/// stay fixed valid values, since `Id::new` can refuse arbitrary text).
/// Two calls with different seeds are the same *kind* of refusal; only
/// their message text differs.
fn build_error(shape: ErrorShape, seed: &str) -> NativeCliError {
    let path = PathBuf::from(format!("/tmp/{seed}"));
    match shape {
        ErrorShape::Usage => NativeCliError::usage(format!("unsupported thing {seed}")),
        ErrorShape::UnsupportedArgumentValue => NativeCliError::UnsupportedArgumentValue {
            flag: "--audience",
            value: seed.to_owned(),
            accepted: vec!["human_review", "ai_agent", "audit", "system", "migration"],
        },
        ErrorShape::Invalid => NativeCliError::invalid(format!("invalid thing {seed}")),
        ErrorShape::StaleRevision => NativeCliError::StaleRevision {
            base_revision_id: id_lossy(&format!("revision:base-{seed}")),
            current_revision_id: id_lossy("revision:current"),
        },
        ErrorShape::StalePlanRevision => NativeCliError::StalePlanRevision {
            plan_id: id_lossy(&format!("plan:{seed}")),
            base_revision_id: id_lossy("revision:plan-base"),
            current_revision_id: id_lossy("revision:current"),
        },
        ErrorShape::GateViolation => NativeCliError::GateViolation {
            message: format!("gate violation {seed}"),
            witness_ids: vec![id_lossy("actor:witness")],
        },
        ErrorShape::StoreIntegrity => {
            NativeCliError::StoreIntegrity(format!("tampered stored gate {seed}"))
        }
        ErrorShape::Core => NativeCliError::Core(
            higher_graphen_core::Id::new(String::new()).expect_err("empty id is always refused"),
        ),
        ErrorShape::StoreIo => NativeCliError::Store(NativeStoreError::Io {
            path: path.clone(),
            source: std::io::Error::other(seed.to_owned()),
        }),
        ErrorShape::StoreJson => NativeCliError::Store(NativeStoreError::Json {
            path: path.clone(),
            source: serde_json::from_str::<Value>("not json").expect_err("malformed by design"),
        }),
        ErrorShape::StoreUnsupportedSchema => {
            NativeCliError::Store(NativeStoreError::UnsupportedSchema {
                path: path.clone(),
                actual: seed.to_owned(),
                expected: "highergraphen.case.space.v1",
            })
        }
        ErrorShape::StoreUnsupportedVersion => {
            NativeCliError::Store(NativeStoreError::UnsupportedVersion {
                path: path.clone(),
                actual: seed.len() as u32,
                expected: 1,
            })
        }
        ErrorShape::StoreMissingCase => NativeCliError::Store(NativeStoreError::MissingCase {
            case_space_id: id_lossy(&format!("case_space:{seed}")),
            path: path.clone(),
        }),
        ErrorShape::StoreExistingCase => {
            NativeCliError::Store(NativeStoreError::ExistingCase { path: path.clone() })
        }
        ErrorShape::StoreLockUnavailable => {
            NativeCliError::Store(NativeStoreError::LockUnavailable {
                path: path.clone(),
                reason: seed.to_owned(),
            })
        }
        ErrorShape::StoreReplayMismatch => {
            NativeCliError::Store(NativeStoreError::ReplayMismatch {
                path: path.clone(),
                reason: seed.to_owned(),
            })
        }
        ErrorShape::StoreInvalidMorphism => {
            NativeCliError::Store(NativeStoreError::InvalidMorphism {
                path,
                reason: seed.to_owned(),
            })
        }
        ErrorShape::Review => NativeCliError::Review(crate::native_review::NativeReviewError {
            message: format!("review error {seed}"),
        }),
        ErrorShape::Eval => NativeCliError::Eval(crate::native_eval::NativeEvalError {
            violations: vec![crate::native_eval::NativeEvalViolation {
                code: crate::native_eval::NativeEvalViolationCode::DanglingReference,
                record_id: None,
                field: "field".to_owned(),
                message: seed.to_owned(),
            }],
        }),
        ErrorShape::Io => NativeCliError::Io {
            path,
            source: std::io::Error::other(seed.to_owned()),
        },
        ErrorShape::Json => {
            NativeCliError::Json(serde_json::from_str::<Value>("not json").expect_err("malformed"))
        }
    }
}

fn id_lossy(value: &str) -> Id {
    Id::new(value.to_owned()).expect("test id")
}

#[test]
fn error_code_is_total_non_empty_and_independent_of_payload_text() {
    arbtest::arbtest(
        |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
            let shape = *u.choose(ALL_ERROR_SHAPES)?;
            let seed_a = u.int_in_range(0_u32..=999)?.to_string();
            let seed_b = u.int_in_range(0_u32..=999)?.to_string();

            let code_a = build_error(shape, &seed_a).error_code();
            let code_b = build_error(shape, &seed_b).error_code();

            assert!(!code_a.is_empty(), "{shape:?} produced an empty error_code");
            assert_eq!(
                code_a, code_b,
                "{shape:?}'s error_code must not depend on message/payload text"
            );
            Ok(())
        },
    );
}

/// The property above only pins that a code is non-empty and stable across
/// message text — it would not notice `"gate_violation"` silently renamed
/// to `"gate_denied"`, since every assertion in it is relative to whatever
/// the code currently is. These strings are a published contract now (the
/// refusal schema, the SKILL.md retry taxonomy, and every integration test
/// that asserts on `error_code` all name them literally), so pin the exact
/// value per shape here, table-driven, in one place a rename has to walk
/// through.
#[test]
fn error_code_strings_are_pinned_exactly() {
    let expected: &[(ErrorShape, &str)] = &[
        (ErrorShape::Usage, "usage"),
        (ErrorShape::UnsupportedArgumentValue, "usage"),
        (ErrorShape::Invalid, "invalid"),
        (ErrorShape::StaleRevision, "stale_revision"),
        (ErrorShape::StalePlanRevision, "stale_plan_revision"),
        (ErrorShape::GateViolation, "gate_violation"),
        (ErrorShape::StoreIntegrity, "store_integrity"),
        (ErrorShape::Core, "invalid_id"),
        (ErrorShape::StoreIo, "store_io"),
        (ErrorShape::StoreJson, "store_io"),
        (ErrorShape::StoreUnsupportedSchema, "unsupported_schema"),
        (ErrorShape::StoreUnsupportedVersion, "unsupported_schema"),
        (ErrorShape::StoreMissingCase, "missing_case_space"),
        (ErrorShape::StoreExistingCase, "existing_case_space"),
        (ErrorShape::StoreLockUnavailable, "lock_unavailable"),
        (ErrorShape::StoreReplayMismatch, "store_integrity"),
        (ErrorShape::StoreInvalidMorphism, "store_integrity"),
        (ErrorShape::Review, "invalid"),
        (ErrorShape::Eval, "evaluation_failed"),
        (ErrorShape::Io, "io_error"),
        (ErrorShape::Json, "invalid"),
    ];
    assert_eq!(
        expected.len(),
        ALL_ERROR_SHAPES.len(),
        "every constructible shape must be pinned here, not only a sample"
    );
    for (shape, expected_code) in expected {
        assert_eq!(
            build_error(*shape, "seed").error_code(),
            *expected_code,
            "{shape:?}"
        );
    }
}

/// Documents, over the three real confinement-refusal message shapes
/// (lexical rejection, canonicalization failure, resolved-but-outside-root
/// — issue #21), that `Invalid`'s `error_code` does not depend on message
/// text. These messages are literals, not the output of `prepare_claim`, so
/// this test alone cannot catch a future subdivision that carves one mode
/// into a new variant — a code-only check would stay green even if
/// production code started constructing a different variant, since these
/// literals never change. The guard that actually catches that is
/// `expect_confined_refusal`'s `Err(NativeCliError::Invalid(message))`
/// **pattern match** (`native_cli/ops/mutations.rs`), which every real
/// confinement test in that module goes through: production code
/// constructing any other variant for one of the three modes fails that
/// match immediately, on the real code path, regardless of what
/// `error_code()` returns for it.
#[test]
fn invalid_error_code_does_not_depend_on_which_confinement_message_produced_it() {
    arbtest::arbtest(
        |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
            let messages = [
                "evidence packet /tmp/p.json was refused: artifact path must not contain `..`",
                "evidence packet /tmp/p.json was refused: artifact path could not be canonicalized",
                "evidence packet /tmp/p.json was refused: artifact path does not resolve inside \
                 the packet's directory",
            ];
            let left = messages[u.choose_index(messages.len())?];
            let right = messages[u.choose_index(messages.len())?];

            assert_eq!(
                NativeCliError::Invalid(left.to_owned()).error_code(),
                NativeCliError::Invalid(right.to_owned()).error_code(),
                "all three confined-artifact refusal modes must share one code"
            );
            Ok(())
        },
    );
}
