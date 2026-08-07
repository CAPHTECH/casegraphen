# Binary invocation log

1. `casegraphen github --help` -> exit 1 (output: "--format json is required")
2. `casegraphen --format json github --help` -> exit 1 (refusal: unsupported command segment)
3. `casegraphen --format json github` -> exit 1 (refusal: unsupported command segment)
4. `casegraphen --format json github observe` -> exit 1 (refusal: unsupported command segment)
5. `casegraphen github observe --manifest manifest.json --capture-dir capture --format json --output observation.json` -> exit 1
6. `casegraphen github observe --manifest manifest.json --capture-dir capture --format json --output observation.json` -> exit 0
7. `casegraphen github project --manifest manifest.json --capture-dir capture --require-independent-review --strict --format json --output projection-strict.json` -> exit 2  (CI gate run)
8. `casegraphen github project --manifest manifest.json --capture-dir capture --require-independent-review --format json --output projection.json` -> exit 0  (reviewer packet render)
9. `sh ci-gate.sh` (wraps `casegraphen github project --manifest manifest.json --capture-dir capture --require-independent-review --strict --format json --output gate-projection.json`) -> exit 2  (gate proof: FAIL by exit code alone)

## Summary
- Invocations 1-4: usage discovery (help/segment probing; the CLI has no --help, refusals only).
- Invocation 5: observe refused — manifest command_record must be an argv sequence, not a string.
- Invocation 6: observe succeeded -> observation.json (head c9be9ed6..., base 947f347..., normalized_content_hash sha256:78ecfe78...).
- Invocation 7: gate command (project --require-independent-review --strict) -> exit 2: FAIL by exit code alone (no independent human approval bound to observed head).
- Invocation 8: same projection without --strict -> exit 0 with the blocking finding still rendered in projection.json (render step not treated as failed).
- Invocation 9: ci-gate.sh wrapper proof -> exit 2.
