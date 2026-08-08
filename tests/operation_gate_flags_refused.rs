//! Issue #164: the seven operation-gate-shaped flags (`--actor-id`,
//! `--capability-id`, `--operation-scope-id`, `--audience`,
//! `--source-boundary-id`, `--gate-profile`, `--gate-profile-file`) parse
//! successfully on every command, because `NativeOptions::consume_arg`
//! recognizes them unconditionally regardless of whether the arm that
//! follows ever calls `resolve_operation_gate_options`. `morphism propose`
//! used to accept and silently drop all five id/audience flags at once
//! (measured on a real store); `binding register` skipped even the
//! `--gate-profile`/`--gate-profile-file` pairing check every gated command
//! enforces. #130 drew this same line for identity flags on `lift`
//! (accepted-and-silently-dropped is worse than an unknown flag, because
//! nothing downstream ever reads it) — this pins the refusal for the
//! gate-flag class on every command issue #164 named as having real operator
//! motive to type them: `morphism propose`, `binding register`, `plan
//! propose`, `memory propose`, `memory index rebuild`, `space new`, and every
//! `lift` adapter.
//!
//! `refuse_operation_gate_flags` runs before any of a command's own required
//! flags (`--store`, `--case-space-id`, `--input`, ...) are checked, so none
//! of these tests need a working store or real input files — a bare `[flag,
//! value]` pair plus `--format json` is enough to reach it.

#![allow(missing_docs)]

use serde_json::Value;
use std::process::{Command, Output};

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

fn stderr_json(output: &Output) -> Value {
    let stderr = stderr(output);
    assert_eq!(stderr.lines().count(), 1, "stderr: {stderr}");
    serde_json::from_str(stderr.trim_end()).expect("stderr refusal JSON")
}

const GATE_FLAGS: [(&str, &str); 7] = [
    ("--actor-id", "actor:garbage"),
    ("--capability-id", "capability:garbage"),
    ("--operation-scope-id", "scope:garbage"),
    ("--audience", "audit"),
    ("--source-boundary-id", "source_boundary:garbage"),
    ("--gate-profile", "orphan"),
    ("--gate-profile-file", "/nonexistent/profile.json"),
];

/// Runs `command_args` once per gate flag — proving each one individually
/// refused, not just "some flag in the set" — then once with all seven
/// together, the shape of the original bug report (`morphism propose` with
/// five gate flags at once, all discarded). Every run asserts: non-zero
/// exit, empty stdout (a refusal never writes a report), `error_code:
/// "usage"` on stderr, and a message naming both the offending flag and
/// `command_label`.
fn assert_gate_flags_refused(command_args: &[&str], command_label: &str) {
    for (flag, value) in GATE_FLAGS {
        let mut args = command_args.to_vec();
        args.push(flag);
        args.push(value);
        let output = run_cli(&args);
        assert!(
            !output.status.success(),
            "{command_label} {flag} succeeded; it must be refused, not silently accepted \
             (stdout: {})",
            stdout(&output)
        );
        assert!(
            stdout(&output).is_empty(),
            "{command_label} {flag}: a refusal must not write a report to stdout: {}",
            stdout(&output)
        );
        let refusal = stderr_json(&output);
        assert_eq!(
            refusal["error_code"].as_str(),
            Some("usage"),
            "{command_label} {flag} must refuse with error_code \"usage\": {refusal}"
        );
        let message = refusal["message"].as_str().expect("usage message");
        assert!(
            message.contains(flag),
            "{command_label} {flag}: refusal message must name the flag it refuses: {message:?}"
        );
        assert!(
            message.contains(command_label),
            "{command_label} {flag}: refusal message must name the command: {message:?}"
        );
    }

    // All seven at once — the shape of the reported bug — names every flag
    // in the message, not only the first one found.
    let mut args = command_args.to_vec();
    for (flag, value) in GATE_FLAGS {
        args.push(flag);
        args.push(value);
    }
    let output = run_cli(&args);
    assert!(
        !output.status.success(),
        "{command_label} with all seven gate flags succeeded; it must be refused (stdout: {})",
        stdout(&output)
    );
    let refusal = stderr_json(&output);
    assert_eq!(
        refusal["error_code"].as_str(),
        Some("usage"),
        "{command_label} all-flags refusal: {refusal}"
    );
    let message = refusal["message"].as_str().expect("usage message");
    for (flag, _) in GATE_FLAGS {
        assert!(
            message.contains(flag),
            "{command_label} all-flags refusal must name {flag}, not just the first one found: \
             {message:?}"
        );
    }
}

#[test]
fn morphism_propose_refuses_gate_flags() {
    assert_gate_flags_refused(
        &["morphism", "propose", "--format", "json"],
        "morphism propose",
    );
}

#[test]
fn plan_propose_refuses_gate_flags() {
    assert_gate_flags_refused(&["plan", "propose", "--format", "json"], "plan propose");
}

#[test]
fn memory_propose_refuses_gate_flags() {
    assert_gate_flags_refused(&["memory", "propose", "--format", "json"], "memory propose");
}

#[test]
fn memory_index_rebuild_refuses_gate_flags() {
    assert_gate_flags_refused(
        &["memory", "index", "rebuild", "--format", "json"],
        "memory index rebuild",
    );
}

#[test]
fn space_new_refuses_gate_flags() {
    assert_gate_flags_refused(&["space", "new", "--format", "json"], "space new");
}

/// The pairing check below (`--gate-profile` without `--gate-profile-file`)
/// is `binding register`'s other defect: every gated command refuses it with
/// "`--gate-profile-file <path> is required with --gate-profile <name>`"
/// (`selected_operation_gate_profile`), but `binding register` never called
/// that function at all, so the malformed pair used to reach input parsing
/// instead. `refuse_operation_gate_flags` runs before
/// `selected_operation_gate_profile` would have, so a lone `--gate-profile`
/// is now refused the same way every other gate flag is — not with the
/// pairing message, since that check still never runs here, but refused
/// nonetheless rather than silently accepted.
#[test]
fn binding_register_refuses_gate_flags() {
    assert_gate_flags_refused(
        &["binding", "register", "--format", "json"],
        "binding register",
    );
}

#[test]
fn binding_register_refuses_gate_profile_alone() {
    let output = run_cli(&[
        "binding",
        "register",
        "--format",
        "json",
        "--gate-profile",
        "orphan",
    ]);
    assert!(!output.status.success());
    assert!(stdout(&output).is_empty());
    let refusal = stderr_json(&output);
    assert_eq!(refusal["error_code"].as_str(), Some("usage"));
    assert!(refusal["message"]
        .as_str()
        .expect("usage message")
        .contains("--gate-profile"));
}

#[test]
fn lift_native_refuses_gate_flags() {
    assert_gate_flags_refused(&["lift", "native", "--format", "json"], "lift");
}

#[test]
fn lift_workflow_refuses_gate_flags() {
    assert_gate_flags_refused(&["lift", "workflow", "--format", "json"], "lift");
}

#[test]
fn lift_case_graph_refuses_gate_flags() {
    assert_gate_flags_refused(&["lift", "case-graph", "--format", "json"], "lift");
}

#[test]
fn lift_github_issues_refuses_gate_flags() {
    assert_gate_flags_refused(&["lift", "github-issues", "--format", "json"], "lift");
}
