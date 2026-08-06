# Binary invocation log

All commands run from $DIR with the binary at ../bin/casegraphen. 3 invocations total, no retries.

## Invocation 1
```
casegraphen github observe --manifest manifest.json --capture-dir . --format json --output observe.json
exit code: 0
```
## Invocation 2 (CI gate, strict)
```
casegraphen github project --manifest manifest.json --capture-dir . --require-independent-review --strict --format json --output project-gate.json
exit code: 2
```
## Invocation 3 (render projection, non-strict)
```
casegraphen github project --manifest manifest.json --capture-dir . --require-independent-review --format json --output project.json
exit code: 0
```
