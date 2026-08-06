//! Guards the one fixture-shaped JSON snippet in the release-decision
//! walkthrough against drifting from the schema it must satisfy — the failure
//! mode behind #106, where the shipped `worker.binding` schema grew a
//! required field the doc's copy-pasteable snippet never picked up. This does
//! not replay the walkthrough (see the doc's own history for why a full
//! literal replay is not attempted in CI); it only proves the one snippet a
//! reader is meant to copy verbatim still satisfies the contract it claims
//! to.

#![allow(missing_docs)]

use std::{fs, path::PathBuf, process::Command};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The value that identifies the `gate.binding.json` snippet from step 3 of
/// the walkthrough: not the doc's Nth fenced block (that breaks the moment a
/// paragraph is added before it) and not the fence's language tag (that
/// breaks if someone writes ` ```json ` instead of ` ```jsonc `), but a value
/// declared inside the snippet itself — the most specific one available.
/// `"schema": "highergraphen.case.workflow.worker_binding.v1"` was
/// considered too, but it is shared by every worker-binding example in this
/// repository (including the shipped `worker.binding.example.json`), so it
/// would stop being unique the moment the walkthrough gained a second
/// binding snippet. `binding_id` names this exact one.
const BINDING_ID_ANCHOR: &str = "\"binding_id\": \"worker_binding:schema-id-gate\"";

/// Every fenced code block in `markdown`, regardless of language tag, in
/// document order.
fn fenced_blocks(markdown: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut search_from = 0;
    while let Some(relative_open) = markdown[search_from..].find("```") {
        let after_open_marker = search_from + relative_open + "```".len();
        let Some(relative_newline) = markdown[after_open_marker..].find('\n') else {
            break; // no fence body follows; nothing left to scan
        };
        let content_start = after_open_marker + relative_newline + 1;
        let Some(relative_close) = markdown[content_start..].find("```") else {
            break; // unterminated fence; treat the rest of the doc as prose
        };
        let content_end = content_start + relative_close;
        blocks.push(&markdown[content_start..content_end]);
        search_from = content_end + "```".len();
    }
    blocks
}

/// Returns the `gate.binding.json` snippet: the one fenced block, of any
/// language, whose content declares [`BINDING_ID_ANCHOR`]. Panics — failing
/// the gate, with the count and the anchor named — unless there is exactly
/// one, since zero or more than one both mean this test no longer knows
/// which snippet it exists to check.
fn extract_gate_binding_snippet(markdown: &str) -> String {
    let matches: Vec<&str> = fenced_blocks(markdown)
        .into_iter()
        .filter(|block| block.contains(BINDING_ID_ANCHOR))
        .collect();
    match matches.len() {
        1 => matches[0].to_owned(),
        0 => panic!(
            "docs/guides/release-decision-walkthrough.md no longer contains a fenced code \
             block declaring {BINDING_ID_ANCHOR} — this test exists to keep the \
             gate.binding.json snippet honest against \
             schemas/casegraphen/worker.binding.schema.json (see #106); the snippet moved, \
             was reworded, or the anchor needs updating to match"
        ),
        n => panic!(
            "docs/guides/release-decision-walkthrough.md now contains {n} fenced code blocks \
             declaring {BINDING_ID_ANCHOR}; this test only knows how to check the one \
             gate.binding.json snippet and needs a more specific anchor to disambiguate them"
        ),
    }
}

#[test]
fn release_decision_walkthrough_binding_snippet_matches_its_schema() {
    let doc_path = root().join("docs/guides/release-decision-walkthrough.md");
    let markdown = fs::read_to_string(&doc_path).expect("read walkthrough");
    let snippet = extract_gate_binding_snippet(&markdown);

    // `<WORK>` and `<REPO>` are the doc's own placeholders for paths a reader
    // substitutes with their real, absolute `$WORK`/`$REPO` before running
    // the command — not part of the field shape being checked. Swapping in
    // any absolute path reproduces exactly what a reader does, without a
    // second validator: `command` and `working_directory` only need to
    // satisfy the schema's absolute-path pattern.
    let instance = snippet
        .replace("<WORK>", "/work")
        .replace("<REPO>", "/repo");

    let instance_path = std::env::temp_dir().join(format!(
        "casegraphen-walkthrough-gate-binding-{}.json",
        std::process::id()
    ));
    let _ = fs::remove_file(&instance_path);
    fs::write(&instance_path, &instance).expect("write extracted snippet");

    let schema_path = root().join("schemas/casegraphen/worker.binding.schema.json");
    let output = Command::new("python3")
        .args(["-m", "jsonschema", "-i"])
        .arg(&instance_path)
        .arg(&schema_path)
        .output()
        .expect("run python3 -m jsonschema");
    assert!(
        output.status.success(),
        "docs/guides/release-decision-walkthrough.md's gate.binding.json snippet no longer \
         satisfies schemas/casegraphen/worker.binding.schema.json (this is exactly the #106 \
         failure mode: a required field the doc's copy-pasteable snippet did not follow):\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
