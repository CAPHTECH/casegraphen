//! Guards the two examples published for issue #123's entry ladder
//! (`docs/guides/entry-ladder.md`) against drifting from what the tool
//! actually does — the same failure mode `tests/walkthrough_conformance.rs`
//! guards for the walkthrough's `gate.binding.json` snippet (#106), plus a
//! stronger check: this file also runs the real binary end to end and
//! asserts on the derived output the guide quotes, not only on schema shape.
//! It also guards the fix to the release-decision genesis (issue #123): the
//! derived `payload`/`genesis_case_space` copy must not come back.

#![allow(missing_docs)]

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_path(relative: &str) -> PathBuf {
    root().join(relative)
}

fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(args)
        .output()
        .expect("run casegraphen CLI")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout utf8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr utf8")
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "casegraphen-entry-ladder-test-{label}-{}-{nanos}",
        std::process::id()
    ))
}

/// Every fenced code block in `markdown`, regardless of language tag, in
/// document order. Copied from `tests/walkthrough_conformance.rs` rather than
/// shared, since neither file is a library the other should depend on for one
/// small function.
fn fenced_blocks(markdown: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut search_from = 0;
    while let Some(relative_open) = markdown[search_from..].find("```") {
        let after_open_marker = search_from + relative_open + "```".len();
        let Some(relative_newline) = markdown[after_open_marker..].find('\n') else {
            break;
        };
        let content_start = after_open_marker + relative_newline + 1;
        let Some(relative_close) = markdown[content_start..].find("```") else {
            break;
        };
        let content_end = content_start + relative_close;
        blocks.push(&markdown[content_start..content_end]);
        search_from = content_end + "```".len();
    }
    blocks
}

/// The one fenced block containing `anchor`. Panics — failing the gate, with
/// the count and the anchor named — unless there is exactly one, since zero
/// or more than one both mean this test no longer knows which snippet it
/// exists to check.
fn extract_snippet<'a>(markdown: &'a str, anchor: &str, doc_label: &str) -> &'a str {
    let matches: Vec<&str> = fenced_blocks(markdown)
        .into_iter()
        .filter(|block| block.contains(anchor))
        .collect();
    match matches.len() {
        1 => matches[0],
        0 => panic!(
            "{doc_label} no longer contains a fenced code block declaring {anchor:?} — the \
             anchor moved, was reworded, or the snippet was removed"
        ),
        n => panic!(
            "{doc_label} now contains {n} fenced code blocks declaring {anchor:?}; this test \
             only knows how to check one and needs a more specific anchor to disambiguate them"
        ),
    }
}

fn entry_ladder_doc() -> String {
    fs::read_to_string(repo_path("docs/guides/entry-ladder.md")).expect("read entry-ladder.md")
}

#[test]
fn entry_ladder_workflow_snippet_matches_the_shipped_example_file() {
    let doc = entry_ladder_doc();
    let snippet = extract_snippet(
        &doc,
        "\"workflow_graph_id\": \"workflow_graph:mini\"",
        "docs/guides/entry-ladder.md",
    );
    let shipped = fs::read_to_string(repo_path(
        "docs/guides/entry-ladder/mini-workflow.graph.json",
    ))
    .expect("read mini-workflow.graph.json");
    assert_eq!(
        snippet.trim_end(),
        shipped.trim_end(),
        "the workflow-graph snippet quoted in docs/guides/entry-ladder.md no longer matches \
         docs/guides/entry-ladder/mini-workflow.graph.json byte-for-byte — a reader copying the \
         doc and a reader downloading the file would get different input"
    );
}

#[test]
fn entry_ladder_examples_satisfy_their_schemas() {
    for (input, schema) in [
        (
            "docs/guides/entry-ladder/mini-workflow.graph.json",
            "schemas/casegraphen/workflow.graph.schema.json",
        ),
        (
            "docs/guides/entry-ladder/mini-genesis.case.space.json",
            "schemas/casegraphen/native.case.space.schema.json",
        ),
    ] {
        let output = Command::new("python3")
            .args(["-m", "jsonschema", "-i"])
            .arg(repo_path(input))
            .arg(repo_path(schema))
            .output()
            .expect("run python3 -m jsonschema");
        assert!(
            output.status.success(),
            "{input} no longer satisfies {schema}:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Rung 1 of the entry ladder: `lift workflow` then `space reason`, over the
/// exact shipped example, through the real binary. Asserts on the derived
/// output the guide quotes verbatim, so a change to readiness derivation, the
/// obstruction message shape, or the lift adapter shows up here rather than
/// only as a stale doc.
#[test]
fn entry_ladder_analysis_loop_reproduces_the_published_transcript() {
    let store = unique_temp_dir("analysis").join("store");

    let lift = run_cli(&[
        "lift",
        "workflow",
        "--store",
        store.to_str().expect("store path"),
        "--input",
        repo_path("docs/guides/entry-ladder/mini-workflow.graph.json")
            .to_str()
            .expect("input path"),
        "--revision-id",
        "revision:mini-genesis",
        "--format",
        "json",
    ]);
    assert!(
        lift.status.success(),
        "lift workflow failed: {}",
        stderr(&lift)
    );
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&lift)).expect("lift workflow report parses");
    let case_space_id = report["result"]["case_space"]["case_space_id"]
        .as_str()
        .expect("result.case_space.case_space_id is a string");
    assert_eq!(
        case_space_id, "case_space:workflow_graph:mini",
        "the entry-ladder guide documents reading the created space id from \
         result.case_space.case_space_id and expects this exact derived value"
    );

    let reason = run_cli(&[
        "space",
        "reason",
        "--store",
        store.to_str().expect("store path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "text",
    ]);
    assert!(
        reason.status.success(),
        "space reason failed: {}",
        stderr(&reason)
    );
    let text = stdout(&reason);
    for expected in [
        "Progress: blocked",
        "- work_item:design",
        "obstruction:unresolved-dependency:work-item-implement:work-item-design",
        "work_item:implement depends on unresolved cell work_item:design.",
    ] {
        assert!(
            text.contains(expected),
            "space reason output no longer contains {expected:?} — the published analysis-loop \
             transcript has drifted:\n{text}"
        );
    }

    fs::remove_dir_all(store.parent().expect("store parent")).ok();
}

/// Rung 2 of the entry ladder: `lift native` then `cell transition`, over the
/// exact shipped minimal genesis, through the real binary.
#[test]
fn entry_ladder_governed_loop_reproduces_the_published_transcript() {
    let store = unique_temp_dir("governed").join("store");
    let case_space_id = "case_space:mini-governed";

    let lift = run_cli(&[
        "lift",
        "native",
        "--store",
        store.to_str().expect("store path"),
        "--input",
        repo_path("docs/guides/entry-ladder/mini-genesis.case.space.json")
            .to_str()
            .expect("input path"),
        "--revision-id",
        "revision:mini-genesis",
        "--format",
        "json",
    ]);
    assert!(
        lift.status.success(),
        "lift native failed: {}",
        stderr(&lift)
    );

    let transition = run_cli(&[
        "cell",
        "transition",
        "--store",
        store.to_str().expect("store path"),
        "--case-space-id",
        case_space_id,
        "--base-revision-id",
        "revision:mini-genesis",
        "--cell-id",
        "work:mini-task",
        "--to",
        "resolved",
        "--actor-id",
        "actor:mini-operator",
        "--capability-id",
        "capability:mini-operator",
        "--operation-scope-id",
        case_space_id,
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:mini-governed",
        "--format",
        "json",
    ]);
    assert!(
        transition.status.success(),
        "cell transition failed: {}",
        stderr(&transition)
    );

    let reason = run_cli(&[
        "space",
        "reason",
        "--store",
        store.to_str().expect("store path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "text",
    ]);
    assert!(
        reason.status.success(),
        "space reason failed: {}",
        stderr(&reason)
    );
    let text = stdout(&reason);
    assert!(
        text.contains("Progress: complete"),
        "the published minimal-governed-loop transcript claims one cell transition reaches \
         Progress: complete; the actual run did not:\n{text}"
    );

    fs::remove_dir_all(store.parent().expect("store parent")).ok();
}

/// Documents the known, separate CLI defect the entry-ladder guide warns
/// readers about (`lift workflow` accepts but ignores `--case-space-id`) so
/// that guide's caution note is checked against real behaviour rather than
/// asserted once and left to rot. If this test starts failing, either the
/// defect was fixed — update the caution note and this test together — or
/// something else changed; either way, the doc must not go on misleading a
/// reader about a flag it silently drops.
#[test]
fn lift_workflow_still_silently_ignores_case_space_id() {
    let store = unique_temp_dir("ignored-flag").join("store");

    let lift = run_cli(&[
        "lift",
        "workflow",
        "--store",
        store.to_str().expect("store path"),
        "--input",
        repo_path("docs/guides/entry-ladder/mini-workflow.graph.json")
            .to_str()
            .expect("input path"),
        "--case-space-id",
        "case_space:ignored-by-lift",
        "--revision-id",
        "revision:mini-genesis",
        "--format",
        "json",
    ]);
    assert!(
        lift.status.success(),
        "lift workflow failed: {}",
        stderr(&lift)
    );
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&lift)).expect("lift workflow report parses");
    let case_space_id = report["result"]["case_space"]["case_space_id"]
        .as_str()
        .expect("result.case_space.case_space_id is a string");
    assert_eq!(
        case_space_id, "case_space:workflow_graph:mini",
        "--case-space-id case_space:ignored-by-lift was passed but the created space id was not \
         derived from it — if this now equals the requested id, the flag was fixed to be \
         honoured (or refused); update docs/guides/entry-ladder.md's caution note accordingly"
    );

    fs::remove_dir_all(store.parent().expect("store parent")).ok();
}

/// Issue #123: the release-decision genesis carried the full derived
/// `payload.added_cells`/`payload.added_relations`/`genesis_case_space` copy
/// that `lift native` overwrites on every import — the walkthrough's own
/// authoring guidance says never to hand-write it. This pins that the fix
/// (deleting the copy from the checked-in file) does not silently come back.
#[test]
fn release_decision_genesis_carries_no_derived_payload_copy() {
    let genesis: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(repo_path(
            "docs/guides/release-decision/genesis.case.space.json",
        ))
        .expect("read release-decision genesis"),
    )
    .expect("release-decision genesis parses");
    let metadata = genesis["morphism_log"][0]["morphism"]["metadata"]
        .as_object()
        .expect("genesis morphism metadata is an object");
    for derived_key in ["payload", "genesis_case_space"] {
        assert!(
            !metadata.contains_key(derived_key),
            "docs/guides/release-decision/genesis.case.space.json's genesis morphism carries a \
             {derived_key:?} key again — this is exactly the derived copy issue #123 removed; \
             lift native overwrites it on every import (write_genesis_materialization), so \
             hand-writing it back only doubles the apparent authoring surface"
        );
    }
}

/// The minimal governed-loop example must never grow the same derived copy —
/// it exists specifically to demonstrate that `case_cells`/`case_relations`
/// are the whole authoring surface.
#[test]
fn mini_genesis_carries_no_derived_payload_copy() {
    let genesis: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(repo_path(
            "docs/guides/entry-ladder/mini-genesis.case.space.json",
        ))
        .expect("read mini genesis"),
    )
    .expect("mini genesis parses");
    let metadata = genesis["morphism_log"][0]["morphism"]["metadata"]
        .as_object()
        .expect("genesis morphism metadata is an object");
    for derived_key in ["payload", "genesis_case_space"] {
        assert!(
            !metadata.contains_key(derived_key),
            "docs/guides/entry-ladder/mini-genesis.case.space.json's genesis morphism carries a \
             {derived_key:?} key — the whole point of this example is that case_cells and \
             case_relations are the only content an author writes"
        );
    }
}
