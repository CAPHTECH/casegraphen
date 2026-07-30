# Running work through a worker

Read the
[worker execution security and approval policy](https://github.com/CAPHTECH/casegraphen/blob/main/docs/security/worker-execution-policy.md)
before enabling a worker against a real project. It is the approval policy, not
background reading.

`$STORE`, `$CS`, and `cur()` below are from SKILL.md.

**This path is for deterministic gate nodes only** — a checker, a linter, a test
command, a version check. It is not the path for LLM or agent nodes: the only
worker kind is `shell`, so a model call means a script carrying an API key
through `env_allowlist`, which is residual risk 3 of that policy. When a runtime
executes the work, read `references/governing.md` instead and record its reports
as evidence.

Order: register the binding → propose the plan → check it → accept it → run one
step. Each step freezes something the next one verifies.

## 1. Binding

```jsonc
{
  "schema": "highergraphen.case.workflow.worker_binding.v1",
  "schema_version": 1,
  "binding_id": "worker_binding:<name>",
  "worker_kind": "shell",
  "command": "/absolute/path/to/gate.sh",
  "args": [],
  "working_directory": "/absolute/path/to/project",
  "resolved_command_path": "ignored — the tool measures this",
  "resolved_working_directory": "ignored — the tool measures this",
  "command_content_hash": "0000000000000000000000000000000000000000000000000000000000000000",
  "env_allowlist": [],
  "timeout_ms": 10000,
  "capability_ids": ["capability:<worker-exec>"]
}
```

Rules the adapter enforces:

- **Name a single pinned program, not an interpreter.** `command` is
  content-hashed. `/bin/sh -c "<script>"` pins `sh`, so the script itself is
  unprotected; a script file as `command` pins the script. Write the script,
  `chmod +x` it, and point `command` at it.
- `command` and `working_directory` must be absolute, and the command must carry
  an execute bit. A file without one is refused before spawning.
- The environment is cleared. `env_allowlist` cannot include `PATH`, any
  `LD_*`/`DYLD_*` variable, or anything in the `CASEGRAPHEN_*` namespace, so
  **call external programs by absolute path inside the script** (`/usr/bin/grep`,
  not `grep`). Paths to project files can be relative to `working_directory`.
- `timeout_ms` is mandatory.
- stdout/stderr are hashed in full and retained up to 4 MiB under
  `runs/<trace>/`.

```sh
casegraphen binding register --store "$STORE" --input binding.json --format json
```

The response shows the measured `resolved_command_path` and
`command_content_hash`. Whatever you declared for those is discarded.

## 2. Plan

One step per work item. `base_revision_id` is the current revision.

```jsonc
{
  "schema": "highergraphen.case.workflow.execution_plan.v1",
  "schema_version": 1,
  "plan_id": "plan:<name>",
  "case_space_id": "<case space id>",
  "base_revision_id": "<current revision>",
  "steps": [{
    "step_id": "step:<name>",
    "work_cell_id": "work:<id>",
    "input_projection_id": null,
    "worker_binding_id": "worker_binding:<name>",
    "success_evidence_requirement_ids": ["evidence:<existing placeholder cell>"],
    "allowed_transition_classes": [
      { "morphism_type": "update", "target_cell_types": ["work"], "to_lifecycles": ["resolved"] }
    ]
  }],
  "provenance": { "source": { "kind": "ai" }, "confidence": 0.8, "review_status": "unreviewed" },
  "review_status": "unreviewed",
  "metadata": {}
}
```

- `success_evidence_requirement_ids` must name **existing evidence cells**; the
  run links its output evidence to them and, if any stays unsatisfied, applies no
  transition. This gates the transition only — it does not affect readiness.
- `allowed_transition_classes` must not be empty, and it is the whole
  pre-authorization: anything outside it becomes an unreviewed proposal with
  `transition_not_authorized`. Keep it narrow.
- `plan check` reports `on_readiness_frontier` per step. A step whose work cell
  is not on the frontier will never dispatch, so fix readiness first.

`plan accept` freezes `plan_content_hash` and the binding hashes. Editing either
file afterwards is detected, and re-registering to "fix" a mismatch throws away
the review — redo propose/accept instead.

## 3. Run exactly one step

```sh
casegraphen run --step --store "$STORE" --case-space-id "$CS" --plan-id <id> \
  --base-revision-id "$(cur)" --actor-id <runner> --enable-worker shell \
  --capability-id <dispatch capability> --capability-id <worker-exec capability> \
  --operation-scope-id "$CS" --audience audit --source-boundary-id <boundary> --format json
```

The dispatch gate must cover every capability the binding declares: a
`capability_ids` entry in the binding that is not among the `--capability-id`
flags fails with `dispatch operation gate does not cover worker binding …`.

`--enable-worker shell` is required on **every** invocation; there is no
persistent setting. Omitting it does not merely refuse: the run directory is
already reserved and a trace anchored, so the revision moves, the attempt is
spent, and the next call needs `--retry-step`. Decide before invoking.

One invocation advances at most one step. Loop by re-reading the revision and
calling again; there is no daemon, scheduler, or retry engine, and adding one is
out of scope.

## 4. Read the outcome

| `status` | Meaning | Next |
|---|---|---|
| `step_executed` | Worker succeeded, requirements satisfied, authorized transition applied | Continue with the new revision |
| `step_failed` | Worker ran and failed, or a requirement stayed unsatisfied. Evidence was attached; no transition | Fix the underlying problem, then `--retry-step` |
| `no_dispatchable_step` | Nothing eligible. Read `step_reasons` | See below |

`step_reasons` explains each ineligible step: `work_cell_missing`,
`work_cell_not_on_frontier`, `work_cell_lifecycle_not_active` (the work cell must
be `active` — `proposed` is not dispatchable), `already_executed`,
`dispatch_in_progress`, or `prior_failed_trace_requires_retry`. A binding or plan
that no longer matches its registration surfaces as a `binding_hash_mismatch` or
`binding_identity_mismatch` obstruction instead, with nothing spawned.

The CLI exits 0 for all of these. A failing gate is a finding.

Worker output enters as an evidence cell with `review_status: unreviewed` and
`evidence_boundary: worker_output`, linked at `diagnostic` strength. "The
command exited 0" is not "the goal is achieved": to let worker output satisfy a
hard requirement, promote it with `review accept`.

Each run writes `runs/<trace>/` containing `execution.trace.json`,
`worker.report.json`, `input.report.json`, `stdout`, and `stderr`, and anchors
the trace hash in the log. When something is disputed, follow trace → report →
log entry → `space validate`.
