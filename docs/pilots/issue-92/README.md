# Issue 92 Memory Plane dogfood pilot

This retained pilot uses CaseGraphen's own ADR 0002 runtime boundary as coding-
agent project memory. It demonstrates exact source capture, a scoped temporal
constraint proposal, policy/query contracts, eight poisoning cases, and a
release report. It does not claim that the proposal is accepted or write it to
a CaseStore.

## Retained artifacts

- `source/adr-0002-runtime-boundary.txt`: immutable source bytes excerpted for
  the pilot;
- `memory.source_record.v0.json`: exact SHA-256 and provenance metadata;
- `memory.claim.v0.json`: unreviewed reusable architecture constraint;
- `memory.policy.v0.json`: bounded coding-agent/audit actor grant;
- `memory.query.v0.json`: current code-change query template;
- `adversarial-corpus.v0.json`: the eight required poisoning/staleness/scope
  cases and their executable test evidence;
- `evaluation-report.v0.json`: retained safety, retrieval, and memory-action
  observations.

Run the retained evidence gate:

```sh
python3 scripts/memory-plane-pilot.py
cargo test --test memory_plane
cargo test --test product_surface operational_memory_tools_are_read_only_or_unreviewed_proposals
```

The source and claim can be checked without mutation:

```sh
casegraphen memory check \
  --input docs/pilots/issue-92/memory.claim.v0.json \
  --source-record docs/pilots/issue-92/memory.source_record.v0.json \
  --source-artifact docs/pilots/issue-92/source/adr-0002-runtime-boundary.txt \
  --policy docs/pilots/issue-92/memory.policy.v0.json \
  --format json
```

The checked claim is still not accepted. A real pilot store must attach the
exact artifact, propose the evidence cell, and use the existing independent
review and gated morphism flow before a normal current query may return it.

## Result

All six safety counters are zero in the bounded regression corpus. Current
queries exclude stale and hard-conflicted claims before ranking; conflict IDs,
omissions, and budget loss remain visible; indexes rebuild equivalently; MCP
proposals cannot inject acceptance and do not move the CaseSpace revision.

Stable promotion is not justified by this bounded corpus. Multi-session coding
tasks with an actual runtime are still required to measure action-constraint
violations, context efficiency against a baseline, reviewer load, and recovery
from changed project decisions.
