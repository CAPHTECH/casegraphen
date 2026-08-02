//! Mechanically checks that every `@carries-test` marker in the design-layer
//! FSL specs under `docs/specs/` still names a test that exists in the
//! source tree.
//!
//! Issue #37: a spec's `fslc verify` result of `proved` is a claim about the
//! model in that file, not evidence about the shipped binary — four times
//! already, a spec here returned `proved` while modelling a machine simpler
//! than the one being built. Each spec now carries a CORRESPONDENCE section
//! naming, per requirement, the real tests that carry its claim into the
//! shipped code (an explicit `NONE` where nothing does, or `PARTIAL` where a
//! real test carries only part of what the tag claims). This file is the
//! only thing that notices when one of those names goes stale — renamed,
//! deleted, or never existed. It asserts nothing about whether the named
//! tests pass; that is `cargo test`'s job.

use std::{fs, path::Path};

const SPEC_FILES: &[&str] = &[
    "docs/specs/operate-halt.fsl",
    "docs/specs/case-lock.fsl",
    "docs/specs/requirement-satisfaction.fsl",
];

/// The marker syntax a spec's CORRESPONDENCE section uses, inside an FSL
/// line comment: `// @carries-test <TAG> <path>::<fn>` names a real test that
/// carries the whole requirement; `// @carries-test <TAG> NONE` states
/// explicitly that nothing carries it; `// @carries-test <TAG> PARTIAL
/// <path>::<fn>` names a real test that carries only part of what the tag
/// claims — the prose above the marker must say which part.
const MARKER: &str = "@carries-test";
const PARTIAL: &str = "PARTIAL";

struct Carry {
    tag: String,
    target: String,
    partial: bool,
    line_number: usize,
}

fn parse_carries(text: &str) -> Vec<Carry> {
    let mut carries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let Some(comment) = line.trim_start().strip_prefix("//") else {
            continue;
        };
        let Some(rest) = comment.trim_start().strip_prefix(MARKER) else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let tag = fields
            .next()
            .unwrap_or_else(|| panic!("line {line_number}: `{MARKER}` has no tag: {line:?}"));
        let second = fields.next().unwrap_or_else(|| {
            panic!("line {line_number}: `{MARKER} {tag}` has no target: {line:?}")
        });
        let (partial, target) = if second == PARTIAL {
            let target = fields.next().unwrap_or_else(|| {
                panic!(
                    "line {line_number}: `{MARKER} {tag} {PARTIAL}` names no test — that is \
                     just `NONE` written misleadingly; write plain `NONE` if nothing carries \
                     this requirement: {line:?}"
                )
            });
            (true, target)
        } else {
            (false, second)
        };
        assert!(
            fields.next().is_none(),
            "line {line_number}: `{MARKER}` has extra fields after the target: {line:?}"
        );
        assert!(
            !(partial && target == "NONE"),
            "line {line_number}: `{MARKER} {tag} {PARTIAL} NONE` is `NONE` written \
             misleadingly through the `{PARTIAL}` form — write plain `NONE` instead: {line:?}"
        );
        carries.push(Carry {
            tag: tag.to_owned(),
            target: target.to_owned(),
            partial,
            line_number,
        });
    }
    carries
}

/// True if `content` defines a function named exactly `fn_name` — matched as
/// a whole identifier rather than a substring, so renaming
/// `a_displaced_holder_refuses_instead_of_appending` to
/// `a_displaced_holder_refuses_instead_of_appending_v2` is caught rather than
/// matched by prefix.
fn defines_fn(content: &str, fn_name: &str) -> bool {
    let needle = format!("fn {fn_name}");
    let mut search_from = 0;
    while let Some(offset) = content[search_from..].find(needle.as_str()) {
        let match_start = search_from + offset;
        let before_is_boundary = content[..match_start]
            .chars()
            .next_back()
            .map_or(true, |c| !c.is_ascii_alphanumeric() && c != '_');
        let after_is_boundary = content[match_start + needle.len()..]
            .chars()
            .next()
            .map_or(true, |c| !c.is_ascii_alphanumeric() && c != '_');
        if before_is_boundary && after_is_boundary {
            return true;
        }
        search_from = match_start + needle.len();
    }
    false
}

#[test]
fn every_carries_test_marker_names_a_test_that_still_exists() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut named_test_count = 0usize;
    let mut none_count = 0usize;
    let mut partial_count = 0usize;

    for spec_relative in SPEC_FILES {
        let spec_path = manifest_dir.join(spec_relative);
        let text = fs::read_to_string(&spec_path)
            .unwrap_or_else(|error| panic!("read {spec_relative}: {error}"));
        let carries = parse_carries(&text);
        assert!(
            !carries.is_empty(),
            "{spec_relative} has no @carries-test markers in its CORRESPONDENCE section"
        );

        for carry in &carries {
            if carry.target == "NONE" {
                none_count += 1;
                continue;
            }
            let Some((rel_path, fn_name)) = carry.target.split_once("::") else {
                panic!(
                    "{spec_relative}:{}: `{MARKER} {}` target {:?} is not `<path>::<fn>` or `NONE`",
                    carry.line_number, carry.tag, carry.target
                );
            };
            let target_path = manifest_dir.join(rel_path);
            let target_content = fs::read_to_string(&target_path).unwrap_or_else(|error| {
                panic!(
                    "{spec_relative}:{}: `{MARKER} {}` names {rel_path}, which does not exist: {error}",
                    carry.line_number, carry.tag
                )
            });
            assert!(
                defines_fn(&target_content, fn_name),
                "{spec_relative}:{}: `{MARKER} {}` names `{fn_name}` in {rel_path}, but no \
                 `fn {fn_name}` is defined there — the test was renamed, deleted, or the \
                 marker never matched a real test",
                carry.line_number,
                carry.tag
            );
            named_test_count += 1;
            if carry.partial {
                partial_count += 1;
            }
        }
    }

    assert!(
        named_test_count > 0,
        "no @carries-test marker across any spec named an actual test"
    );
    assert!(
        none_count > 0,
        "expected at least one honest `NONE` marker across the three specs \
         (issue #37: an explicit \"nothing carries this\" is expected here, \
         not merely possible)"
    );
    assert!(
        partial_count > 0,
        "expected at least one honest `PARTIAL` marker across the three specs \
         (a test that carries only part of what its tag claims is expected here, \
         not merely possible)"
    );
}
