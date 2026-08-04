# ADR 0012: ADR identifiers form a deterministic inventory

## Status

Accepted on 2026-08-04. Resolves issue #82.

## Context

Decision identifiers are authority-bearing references for implementation,
reviews, Skills, guides, and audits. Two independent decisions had reused 0020,
and two had reused 0023. The directory also skipped 0012. Filename prefixes and
headings were maintained manually, so a link could keep resolving while its
prose identity silently referred to a different decision.

## Decision

ADR identifiers are unique, contiguous four-digit numbers beginning at 0001.
The filename is `NNNN-slug.md`, and its first line is exactly an
`# ADR NNNN: ...` heading with the same identifier. Once assigned, an identifier
is not reused for another decision.

The release gate inventories every ADR, rejects duplicate or missing numbers,
rejects filename/heading disagreement, and resolves every relative Markdown
link whose target is an ADR. Negative fixtures independently preserve each
refusal. The README records the next available identifier; after adding an ADR,
its author advances that value in the same change.

To restore a unique contiguous inventory without changing decision content,
the streaming-order decision formerly numbered 0020 becomes ADR 0024, and the
edge-handoff-completeness decision formerly numbered 0023 becomes ADR 0025.
The earlier product-surface ADR 0020 and deployment-authority ADR 0023 retain
their identifiers.

## Consequences

- `ADR NNNN` has one repository-wide meaning.
- Renames and references must be updated atomically.
- A deleted decision must remain as a status-bearing ADR record rather than
  creating a number gap.
- The next available identifier after this repair is 0026.

## Rejected alternatives

- Permit gaps or duplicate numbers through an allowlist: this makes absence and
  reuse policy depend on mutable checker configuration.
- Check headings only: links can still target a differently numbered filename.
- Check links only: two valid files can still claim the same decision identity.
