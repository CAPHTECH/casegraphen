# Binary invocation log (all commands run from $DIR)

1. `casegraphen github observe --manifest manifest.json --capture-dir . --format json --output observe.json` — exit 0
2. `casegraphen github project --manifest manifest.json --capture-dir . --require-independent-review --strict --format json --output project-gate.json` — exit 2
3. `casegraphen github project --manifest manifest.json --capture-dir . --require-independent-review --format json --output project.json` — exit 0
