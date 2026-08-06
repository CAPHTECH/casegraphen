# Issue 88 allocator checkpoint and compaction pilot

This pilot exercises the real allocator at a bounded journal size and retains
latency, RSS, checkpoint size, independent verification, compaction, suffix
replay, full-replay equivalence, crash-boundary, concurrency, release, and
supersede observations. It never supplies reviewed deployment authority and
never accepts runtime output.

```sh
cargo run --release --example resource_allocator_durability_pilot -- \
  docs/pilots/issue-88/resource-allocator-512.report.json

cargo build --locked --release --example resource_allocator_durability_pilot

python3 scripts/resource-allocator-release-scale-pilot.py \
  --binary target/release/examples/resource_allocator_durability_pilot \
  --event-target 10000 \
  --evidence-class release-candidate \
  --require-clean-source \
  --output docs/pilots/issue-88/resource-allocator-10000.report.json

python3 scripts/resource-allocator-release-scale-pilot.py \
  --binary target/release/examples/resource_allocator_durability_pilot \
  --event-target 100000 \
  --evidence-class release-candidate \
  --require-clean-source \
  --output docs/pilots/issue-88/resource-allocator-100000.report.json
```

The 512 lane is the bounded correctness/CI observation. The 10k and 100k lanes
use the same long-lived `UnreviewedResourceJournal` public API shape as the
operational host; direct event-file generation is not release evidence. The
envelope samples process RSS every 500 ms and binds the report to the binary,
harness, allocator source, pilot source, source revision, and host platform.
It exercises immediate release, a shared-read all-active set, and mixed
release/reserve churn through the public allocator API. The retained 10k and
100k reports each exercise exactly 10,000 simultaneously active shared readers;
their mixed-churn lanes execute 1,024 and 4,096 release/reserve pairs,
respectively. Reports always carry `promotion_authority: false`; a clean source
revision and retained binary provenance are necessary but not equivalent to an
independently attested promotion decision.

The accepted fleet budget is explicit and fails the pilot:

| lane | total append | reserve/release pair p95 | restart replay | checkpoint/verify/compaction | checkpoint bytes | RSS |
|---:|---:|---:|---:|---:|---:|---:|
| 512 | 60 s | 100 ms | 5 s | 120 s each | 2,048 bytes/event + 1 MiB | bounded CI only |
| 10k | 300 s | 100 ms | 30 s | 120 s each | 2,048 bytes/event + 1 MiB | 2 GiB |
| 100k | 2,400 s | 100 ms | 120 s | 600 s each | 2,048 bytes/event + 1 MiB | 8 GiB |

## Retained clean-revision results

Both scale reports were generated from exact clean source revision
`9b23383463cb1f1fafb666e7fb87a596b3e090e2` with the same retained binary hash.
They passed every configured threshold while remaining unattested
release-candidate evidence with `promotion_authority: false`.

| lane | append | pair p95 | restart | checkpoint create / verify / compact | checkpoint bytes | peak RSS | all-active / mixed churn |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 10k | 101.5 s | 21 ms | 401 ms | 587 ms / 504 ms / 3.8 s | 18,073,442 | 453,099,520 | 10,000 / 1,024 pairs |
| 100k | 1,040.2 s | 22 ms | 4.9 s | 6.5 s / 5.6 s / 41.9 s | 181,778,445 | 3,751,116,800 | 10,000 / 4,096 pairs |

Promotion requires all three reports, exact full/suffix replay equivalence, no
integrity refusal on valid input, every tamper/crash negative fixture failing
closed, and all latency, checkpoint-size, and memory thresholds. A fast
checkpoint cannot compensate for changed allocator decisions.

The hot path keeps a process-local validated replay state behind a
cross-process writer lock. The `.allocator-head-hint` is only an invalidation
token: missing, malformed, stale, or rolled-back bytes force canonical replay,
and the next authoritative active/archive sequence prevents a stale hint from
hiding a committed event. Startup, checkpoint verification, and audit recovery
still use canonical replay; the event/archive chain remains authority.
The journal directory must be private to one service identity. Unsupported
in-place modification of an older event is detected by restart/full replay and
checkpoint audit; it is not made an O(1) hot-path check by treating filesystem
metadata as cryptographic authority.
