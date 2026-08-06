# Binary invocation log

$BIN = /private/tmp/claude-501/-Users-rizumita-Workspace-casegraphen/3c964d9b-0250-4c0a-b985-e7090f8a4f0c/scratchpad/ablation/bin/casegraphen
$DIR = /private/tmp/claude-501/-Users-rizumita-Workspace-casegraphen/3c964d9b-0250-4c0a-b985-e7090f8a4f0c/scratchpad/ablation/sham2

| # | Command | Exit | Note |
|---|---------|------|------|
| 1 | `$BIN github --help` | 1 | refusal: `--format json is required` |
| 2 | `$BIN github --help --format json` | 1 | refusal: `unsupported github command` |
| 3 | `$BIN github observe --format json` | 1 | refusal: `--manifest <path> is required` |
| 4 | `$BIN github observe --format json --manifest $DIR/manifest.json` | 1 | refusal: `--capture-dir <path> is required` |
| 5 | `$BIN github observe --format json --manifest $DIR/manifest.json --capture-dir $DIR/capture` | 1 | io_error: manifest.json did not exist yet |
| 6 | same, manifest = `{}` | 1 | invalid: missing field `schema` |
| 7 | same, manifest = `{"schema":"x"}` | 1 | invalid: missing field `repository` |
| 8 | same, manifest with unknown probe field | 1 | invalid: unknown field; expected `schema, repository, issue_numbers, pr_number, captured_at, capture_tool, entries` |
| 9 | same, entries[0] unknown probe field | 1 | invalid: entry fields are `category, issue_number, artifact_path, content_hash, command_record` |
| 10 | same, entries[0].category = "zzz" | 1 | invalid: category variants `issue, pr, files, reviews, review_threads, commits, checks` |
| 11 | same, full manifest, command_record as string | 1 | invalid: `command_record` expects a sequence (argv array) |
| 12 | same, argv arrays, schema `casegraphen.github.capture_manifest.v0` | 1 | invalid: expected schema `casegraphen.experimental.github.capture_manifest.v0` |
| 13 | same, corrected schema id | 1 | contract validation: `invalid_category_count` — exactly one Files entry required, found 0 |
| 14 | same, with `files` entry pointing at pr-101.json (which carries `files`) | 0 | SUCCESS — normalized observation written to $DIR/observation.json |
| 15 | `$BIN github gate --format json` | 1 | refusal: `unsupported github command` (no gate subcommand; gating is a `project` flag) |
| 16 | `$BIN github observe --format json --manifest $DIR/manifest.json --capture-dir $DIR/capture --zzz-probe` | 1 | refusal: `unsupported native argument "--zzz-probe" for github` |
| 17 | `$BIN github project --manifest $DIR/manifest.json --capture-dir $DIR/capture --format json --output $DIR/projection.json` | 0 | Task 3: projection rendered for human reviewer while policy unmet; step NOT failed. Residual risk `no_independent_human_approval` carried in the report body only. |
| 18 | `$BIN github project --manifest $DIR/manifest.json --capture-dir $DIR/capture --require-independent-review --format json --output $DIR/gate-projection.json` | 0 | Policy flag alone adds blocking finding `independent_review_policy:...#101@c9be9ed...` to the report but keeps exit 0 (findings audience). |
| 19 | `$BIN github project --manifest $DIR/manifest.json --capture-dir $DIR/capture --require-independent-review --strict --format json --output $DIR/gate-strict-projection.json` | 2 | `--strict` promotes the unmet independent-review policy to the exit code. Projection artifact still written. |
| 20 | `$DIR/ci-gate.sh` (wraps invocation 19 verbatim) | 2 | Task 2 proof: wired CI gate FAILS via exit code alone (exit 2), no JSON parsing. |
