#![allow(missing_docs)]

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const USAGE: &str = include_str!("../src/cli_usage.txt");

#[test]
fn documented_commands_are_accepted() {
    let mut failures = Vec::new();

    for path in usage_paths() {
        let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
            .args(&path)
            .output()
            .unwrap_or_else(|error| panic!("run `casegraphen {}`: {error}", path.join(" ")));
        let label = format!("casegraphen {}", path.join(" "));
        let stderr = String::from_utf8_lossy(&output.stderr);
        let is_version =
            matches!(path.as_slice(), [version] if version == "version" || version == "--version");

        if is_version {
            if !output.status.success() {
                failures.push(format!("{label} should succeed, but stderr was:\n{stderr}"));
            }
            continue;
        }

        if output.status.success() {
            failures.push(format!(
                "{label} unexpectedly succeeded without its required arguments"
            ));
        } else if stderr.contains("unsupported") {
            failures.push(format!("{label} was rejected as unsupported:\n{stderr}"));
        } else if !stderr.contains("required") && !stderr.contains("requires") {
            failures.push(format!(
                "{label} did not fail with a missing-argument usage error:\n{stderr}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "documented command paths that do not reach the parser:\n{}",
        failures.join("\n\n")
    );
}

#[test]
fn accepted_commands_are_documented() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cli = read(&manifest_dir.join("src/cli.rs"));
    let parser = read(&manifest_dir.join("src/native_cli/parser.rs"));
    let documented = usage_paths();
    let namespaces = top_level_namespaces(&cli);
    let mut missing = Vec::new();

    for namespace in namespaces {
        if !documented
            .iter()
            .any(|path| path.first() == Some(&namespace))
        {
            missing.push(format!("casegraphen {namespace}"));
        }

        // `run` and `operate` dispatch on parsed flags rather than an
        // operation string. Assertion 1 proves every documented flag shape
        // reaches its dispatcher.
        if namespace == "run" || namespace == "operate" {
            continue;
        }

        let function = format!("parse_{}", namespace.replace('-', "_"));
        for operation in dispatch_operations(&parser, &function) {
            let path = vec![namespace.clone(), operation];
            if !documented.contains(&path) {
                missing.push(format!("casegraphen {}", path.join(" ")));
            }
        }
    }

    missing.sort();
    assert!(
        missing.is_empty(),
        "parser accepts command paths absent from src/cli_usage.txt:\n{}",
        missing.join("\n")
    );
}

#[test]
fn readme_command_surface_is_documented_in_usage() {
    let readme = read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"));
    let documented = usage_paths();
    let readme_paths = readme_command_paths(&readme);
    let missing = readme_paths
        .difference(&documented)
        .map(|path| format!("casegraphen {}", path.join(" ")))
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "README.md command paths absent from src/cli_usage.txt:\n{}",
        missing.join("\n")
    );
}

#[test]
fn native_report_labels_name_documented_commands() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let documented = usage_paths();
    let mut source_files = vec![
        manifest_dir.join("src/native_cli.rs"),
        manifest_dir.join("src/native_cli_reporting.rs"),
    ];
    collect_rust_files(&manifest_dir.join("src/native_cli"), &mut source_files);

    // These labels describe internal sub-operations performed by `run --step`,
    // not separately dispatchable CLI commands.
    let sub_operation_labels = BTreeSet::from([
        "casegraphen run --step evidence attach",
        "casegraphen run --step trace anchor",
        "casegraphen run --step transition",
    ]);
    let mut missing = BTreeSet::new();

    for source_file in source_files {
        for label in rust_string_literals(&read(&source_file)) {
            if !label.starts_with("casegraphen ")
                || label.contains('{')
                || sub_operation_labels.contains(label.as_str())
            {
                continue;
            }
            let path = label
                .split_whitespace()
                .skip(1)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if !documented.contains(&path) {
                missing.insert(label);
            }
        }
    }

    assert!(
        missing.is_empty(),
        "native report labels that do not name documented commands:\n{}",
        missing.into_iter().collect::<Vec<_>>().join("\n")
    );
}

fn usage_paths() -> BTreeSet<Vec<String>> {
    USAGE
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.starts_with("casegraphen ").then_some(line)
        })
        .flat_map(expand_command_clause)
        .collect()
}

fn readme_command_paths(readme: &str) -> BTreeSet<Vec<String>> {
    let command_surface = readme
        .split_once("## Command surface")
        .map(|(_, command_surface)| command_surface)
        .expect("README.md must contain a `## Command surface` section");
    let mut in_fence = false;
    let mut found_fence = false;
    let mut paths = BTreeSet::new();

    for line in command_surface.lines() {
        if line.trim_start().starts_with("```") {
            if in_fence {
                return paths;
            }
            in_fence = true;
            found_fence = true;
            continue;
        }
        if !in_fence {
            continue;
        }

        let command_line = line.split('#').next().unwrap_or_default().trim();
        for (index, clause) in command_line.split(" | ").enumerate() {
            let clause = clause.trim();
            if clause.is_empty() {
                continue;
            }
            let command = if index == 0 {
                clause.to_owned()
            } else {
                format!("casegraphen {clause}")
            };
            paths.extend(expand_command_clause(&command));
        }
    }

    assert!(
        found_fence,
        "README.md `## Command surface` must contain a fenced command block"
    );
    panic!("README.md command-surface fence is not closed");
}

fn expand_command_clause(command: &str) -> Vec<Vec<String>> {
    let mut tokens = command.split_whitespace();
    assert_eq!(
        tokens.next(),
        Some("casegraphen"),
        "command surface line must start with `casegraphen`: {command}"
    );

    let mut path_tokens = Vec::new();
    for token in tokens {
        if token.starts_with('<') || token.starts_with('[') || token.starts_with('(') {
            break;
        }
        if token.starts_with("--") {
            if path_tokens.len() <= 1 {
                path_tokens.push(token);
            } else {
                break;
            }
        } else {
            path_tokens.push(token);
        }
    }
    assert!(
        !path_tokens.is_empty(),
        "command surface line has no command path: {command}"
    );

    let mut paths = expand_path_tokens(&path_tokens, command);
    // A namespace with no operation word at all (`operate`, the same shape
    // `run` is) has no bare second token to stop this loop at one, so the
    // first ordinary flag (`--store`) gets absorbed above as if it might be
    // an operation marker — the way `--step`/`--frontier` genuinely are for
    // `run`. Unlike `run`, `operate` has only one mode, and its report label
    // (`native_cli/ops/run.rs`'s `operate` function) names only the bare
    // namespace. When the second token is a flag, the namespace alone is
    // therefore documented too, alongside the longer, coincidentally-absorbed
    // path — this only adds paths `usage_paths()` accepts, it never narrows
    // what `documented_commands_are_accepted` exercises against the real
    // binary.
    if path_tokens.len() >= 2 && path_tokens[1].starts_with("--") {
        paths.extend(expand_path_tokens(&path_tokens[..1], command));
    }
    paths
}

fn expand_path_tokens(path_tokens: &[&str], command: &str) -> Vec<Vec<String>> {
    let mut paths = vec![Vec::new()];
    for token in path_tokens {
        let alternatives = token.split('|').collect::<Vec<_>>();
        assert!(
            alternatives
                .iter()
                .all(|alternative| !alternative.is_empty()),
            "empty command alternation in: {command}"
        );
        paths = paths
            .into_iter()
            .flat_map(|path| {
                alternatives.iter().map(move |alternative| {
                    let mut expanded = path.clone();
                    expanded.push((*alternative).to_owned());
                    expanded
                })
            })
            .collect();
    }
    paths
}

fn top_level_namespaces(cli: &str) -> BTreeSet<String> {
    let alternation = delimited_body_after(cli, "segment @", '(', ')')
        .expect("src/cli.rs must contain the top-level `segment @ (...)` dispatch alternation");
    let namespaces = rust_string_literals(alternation)
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert!(
        !namespaces.is_empty(),
        "top-level CLI extraction found zero namespaces; refusing a vacuous surface check"
    );
    namespaces
}

fn dispatch_operations(parser: &str, function: &str) -> BTreeSet<String> {
    let body = function_body(parser, function);
    let mut operations = [
        "match operation.to_str()",
        "match operation",
        "match adapter",
    ]
    .iter()
    .find_map(|selector| {
        delimited_body_after(body, selector, '{', '}').map(match_arm_string_literals)
    })
    .unwrap_or_default();

    if operations.is_empty() {
        for marker in [
            "operation.to_str() != Some(\"",
            "operation.to_str() == Some(\"",
        ] {
            if let Some(after_marker) = body.split_once(marker).map(|(_, rest)| rest) {
                if let Some((operation, _)) = after_marker.split_once('"') {
                    operations.insert(operation.to_owned());
                }
            }
        }
    }

    assert!(
        !operations.is_empty(),
        "parser surface extraction found zero operation arms in `{function}`; \
         refusing a vacuous accepted-implies-documented check"
    );
    operations
}

fn function_body<'a>(source: &'a str, function: &str) -> &'a str {
    let signature = format!("fn {function}(");
    let function_start = source
        .find(&signature)
        .unwrap_or_else(|| panic!("src/native_cli/parser.rs is missing `{signature}...`"));
    delimited_body_after(&source[function_start..], &signature, '{', '}')
        .unwrap_or_else(|| panic!("could not isolate body of `{function}`"))
}

fn match_arm_string_literals(match_body: &str) -> BTreeSet<String> {
    let bytes = match_body.as_bytes();
    let mut strings = BTreeSet::new();
    let mut pattern_start = 0;
    let mut index = 0;
    let mut parentheses = 0_u32;
    let mut brackets = 0_u32;
    let mut braces = 0_u32;
    let mut in_arm = false;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = string_end(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'/') => index = line_comment_end(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = block_comment_end(bytes, index),
            b'(' => parentheses += 1,
            b')' => parentheses = parentheses.saturating_sub(1),
            b'[' => brackets += 1,
            b']' => brackets = brackets.saturating_sub(1),
            b'{' => braces += 1,
            b'}' => {
                braces = braces.saturating_sub(1);
                if in_arm && parentheses == 0 && brackets == 0 && braces == 0 {
                    pattern_start = index + 1;
                    in_arm = false;
                }
            }
            b'=' if bytes.get(index + 1) == Some(&b'>')
                && parentheses == 0
                && brackets == 0
                && braces == 0 =>
            {
                strings.extend(rust_string_literals(&match_body[pattern_start..index]));
                in_arm = true;
                index += 1;
            }
            b',' if in_arm && parentheses == 0 && brackets == 0 && braces == 0 => {
                pattern_start = index + 1;
                in_arm = false;
            }
            _ => {}
        }
        index += 1;
    }
    strings
}

fn delimited_body_after<'a>(
    source: &'a str,
    marker: &str,
    open: char,
    close: char,
) -> Option<&'a str> {
    let marker_start = source.find(marker)?;
    let after_marker = marker_start + marker.len();
    let open_offset = source[after_marker..].find(open)?;
    let open_index = after_marker + open_offset;
    let close_index = matching_delimiter(source, open_index, open as u8, close as u8)?;
    Some(&source[open_index + 1..close_index])
}

fn matching_delimiter(source: &str, open_index: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0_u32;
    let mut index = open_index;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = string_end(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'/') => index = line_comment_end(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = block_comment_end(bytes, index),
            byte if byte == open => depth += 1,
            byte if byte == close => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn rust_string_literals(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut strings = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let end = string_end(bytes, index);
                assert!(end < bytes.len(), "unterminated Rust string literal");
                strings.push(source[index + 1..end].to_owned());
                index = end;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => index = line_comment_end(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = block_comment_end(bytes, index),
            _ => {}
        }
        index += 1;
    }
    strings
}

fn string_end(bytes: &[u8], opening_quote: usize) -> usize {
    let mut index = opening_quote + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return index,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn line_comment_end(bytes: &[u8], opening_slash: usize) -> usize {
    bytes[opening_slash..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| opening_slash + offset)
}

fn block_comment_end(bytes: &[u8], opening_slash: usize) -> usize {
    let mut index = opening_slash + 2;
    while index + 1 < bytes.len() {
        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("read native CLI source entry").path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}
