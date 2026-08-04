# Issue 88 allocator checkpoint and compaction pilot

This pilot exercises the real allocator at a bounded journal size and retains
latency, RSS, checkpoint size, independent verification, compaction, suffix
replay, full-replay equivalence, crash-boundary, concurrency, release, and
supersede observations. It never supplies reviewed deployment authority and
never accepts runtime output.

```sh
cargo run --release --example resource_allocator_durability_pilot -- \
  docs/pilots/issue-88/resource-allocator-512.report.json

CASEGRAPHEN_ALLOCATOR_EVENT_TARGET=10000 \
cargo run --release --example resource_allocator_durability_pilot -- \
  docs/pilots/issue-88/resource-allocator-10000.report.json

CASEGRAPHEN_ALLOCATOR_EVENT_TARGET=100000 \
cargo run --release --example resource_allocator_durability_pilot -- \
  docs/pilots/issue-88/resource-allocator-100000.report.json
```

The 512 lane is the bounded correctness/CI observation. The 10k and 100k lanes
are release evidence and must run on a dedicated host with machine identity and
toolchain metadata retained alongside the report. Promotion requires all three
reports, exact full/suffix replay equivalence, no integrity refusal on valid
input, every tamper/crash negative fixture failing closed, and reviewed latency
and memory thresholds. A fast checkpoint cannot compensate for changed
allocator decisions.
