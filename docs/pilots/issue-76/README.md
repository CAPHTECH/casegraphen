# Issue 76 runtime-family evidence

This directory retains the 2026-08-04 local execution of four materially
different runtime families through the operational MCP host. It is release
evidence for experimental v0, not accepted CaseGraphen evidence.

- `process-jsonl.complete.jsonl` is emitted directly by local subprocesses.
- `file-drop.complete.jsonl` is normalized from isolated workspace files.
- `sqlite-queue.complete.jsonl` is normalized from a transactional durable
  SQLite queue and result table.
- `async-stream.complete.jsonl` is normalized from an asyncio subprocess event
  stream with explicit logical chunk order.
- `pilot-report.json` records canonical lint/reconciliation results and the
  executable assertions.
- `promotion-report.json` separates observed evidence, unknowns, and blockers;
  `promotion_recommended` and `accepted` remain false.
- `retained-evidence.manifest.json` binds all generated reports and JSONL
  streams to their SHA-256 digest and byte length.

The SQLite and asyncio families reserve a declared resource through the
operational allocator, ingest content-addressed artifact bytes through the
generic JSONL boundary, and reconcile the exact resource-expectation bundle.
All successful paths halt at `needs_review`; every failure output is an
unreviewed audit/redesign proposal and cannot mutate accepted topology.

Reproduce after building the host with Rust 1.80:

```sh
rustup run 1.80.0 cargo build --bin casegraphen-mcp-host
python3 scripts/runtime-integration-pilots.py \
  --repo . \
  --host-bin target/debug/casegraphen-mcp-host \
  --output /tmp/casegraphen-issue-76-pilot
```

Runtime duration and environment metadata are observations and may differ.
Verify generated evidence against the new output's own retention manifest;
do not expect the checked-in manifest hash to reproduce across machines.
