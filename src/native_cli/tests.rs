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
