## Invocation 1
```
casegraphen github observe --help
```
exit: 1

## Invocation 2
```
casegraphen github observe --capture-dir capture
```
exit: 

## Invocation 3
```
casegraphen github observe --capture-dir capture --format json
```
exit: 1

## Invocation 4
```
casegraphen github observe --capture-dir capture --manifest manifest.json --format json
```
exit: 1

## Invocation 5
```
casegraphen github observe --capture-dir capture --manifest manifest.json --format json  # manifest={}
```
exit: 1

## Invocation 6
```
casegraphen github observe ... # manifest={"schema":"x"}
```
exit: 1

## Invocation 7
```
casegraphen github observe ... # manifest guess with files map
```
exit: 1

## Invocation 8
```
casegraphen github observe ... # manifest with empty entry
```
exit: 1

## Invocation 9
```
casegraphen github observe ... # entry {category:"x"}
```
exit: 1

## Invocation 10
```
casegraphen github observe ... # entry {category:"pr"}
```
exit: 1

## Invocation 11
```
casegraphen github observe ... # entry {category, artifact_path}
```
exit: 1

## Invocation 12
```
casegraphen github observe ... # entry + content_hash sha256:<hex>
```
exit: 1

## Invocation 13
```
casegraphen github observe ... # + command_record {}
```
exit: 1

## Invocation 14
```
casegraphen github observe ... # command_record as argv list
```
exit: 1

## Invocation 15
```
casegraphen github observe --capture-dir capture --manifest manifest.json --format json --output observation.json  # full manifest, all 6 entries
```
exit: 1

## Invocation 16
```
casegraphen github observe ... # + files entry aliasing pr-101.json
```
exit: 1

## Invocation 17
```
casegraphen github observe ... # + issue_number on issue entry
```
exit: 0

## Invocation 18
```
casegraphen github project --capture-dir capture --manifest manifest.json --format json --output projection.json --require-independent-review
```
exit: 0

## Invocation 19
```
casegraphen github project ... --require-independent-review bogus
```
exit: 1

## Invocation 20
```
casegraphen github project ... --format bogus --require-independent-review
```
exit: 1

## Invocation 21
```
casegraphen github project --capture-dir capture --manifest manifest.json --format json --require-independent-review  # no --output, stdout discarded
```
exit: 0

## Invocation 22
```
casegraphen github
```
exit: 1

## Invocation 23
```
casegraphen github frobnicate
```
exit: 1

## Invocation 24
```
casegraphen github refresh --capture-dir capture --manifest manifest.json --previous-observation observation.json --format json --output refresh.json
```
exit: 1

## Invocation 25
```
casegraphen github refresh --capture-dir capture --manifest manifest.json --previous-capture-dir capture --previous-manifest manifest.json --previous-observation observation.json --format json --output refresh.json
```
exit: 1

## Non-binary steps
- Wrote manifest.json (schema casegraphen.experimental.github.capture_manifest.v0) by iterating on refusal messages; content hashes computed with python3 hashlib (sha256:<hex>).
- Extracted result.pr_observation -> pr-observation.json and result.projection -> reviewer-projection.json with python3 (no binary invocation).
- Wrote answers.md and ci-gate.sh.

## Summary
- 25 binary invocations total (cap reached).
- observe succeeded on invocation 17; project (gated) on 18; refresh explored on 24-25 (needs --previous-manifest and a bare pr_observation record as --previous-observation).
- Key finding: `github project --require-independent-review` records blocking finding `independent_review_policy:...` with result.accepted=false but exits 0, so a purely exit-code CI gate does not fail on this baseline binary.
