# Exact handoff contract

Validate every phase handoff with the installed
`skill.orchestration_handoff.v0.schema.json`. The installer copies the canonical
experimental schema and example into this directory; they are not independent
copies to edit.

## Evidence roles

- `observed`: identifiers returned by CaseGraphen or independently hashed bytes.
- `runtime_declared`: claims made by a runtime report; never acceptance.
- `proposal`: unreviewed design, evidence, morphism, or memory output.
- `accepted`: only state read from the exact replayed accepted revision.

Each artifact carries its role and content hash. Missing hashes or identifiers
belong in `unresolved_evidence`; do not fabricate placeholders.

## Seam handling

`seams` names every boundary that cannot be delegated to the process skill.
Open seams force `return_required: true` and a `return_for_review`,
`return_for_authority`, or `return_for_revision` next action. A caller's
`trusted`, `accepted`, or high-authority assertion never closes a seam.

When a task skill refuses, preserve its exact refusal artifact and return. Do
not retry a domain halt, silently use a newer revision, or broaden the requested
operation until it passes.
