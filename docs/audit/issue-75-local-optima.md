# Issue #75 implementation local-optima audit

## Scope and evidence

- `B`: execution-topology acceptance constructor and its CLI/MCP-visible error
  and canonical review record.
- `M`: invalid topology cannot be accepted; reviewer advice remains nonbinding.
- `N`: experimental review constructor/metadata/tests, not the canonical
  topology validator itself.
- `T`: proposal acceptance, rejection/reopen, replay, and later compilation.

Evidence planes:

1. Code: `execution_topology_review_morphism` calls
   `validate_execution_topology`; graph lint is invoked only after deterministic
   validation and only for accepted content.
2. Behavioral: tests cover unknown node, invalid data binding, self-edge,
   resource mismatch, undeclared policy reference, warning-only acceptance,
   reject auditability, and reviewed compilation.

## Observations

- [Evidence] Accept refuses before a morphism is constructed when canonical
  validation returns findings.
- [Evidence] Finding codes and JSON paths originate from the shared canonical
  validator; no semantic rule was copied into `native_review`.
- [Evidence] Reject/reopen skip semantic acceptance validation but retain the
  same content/revision/artifact/policy-manifest binding.
- [Evidence] Heuristic graph-lint findings are stored separately as
  `execution_topology_review_advisories` and cannot cause refusal.
- [Inference] CLI and delegated MCP hosts receive stable codes/paths through the
  canonical review error; successful review records preserve advisory class.

## Local rationality and compensation halo

Deserializing the typed topology was locally reasonable because it proved wire
shape and enabled hashing. It left a boundary inversion: semantic validity was
checked by later lint/compiler consumers after review authority already
existed. Those downstream refusals formed the compensation halo.

The first corrected version returned only the first deterministic finding.
That was safe but optimized constructor simplicity at the cost of repeated
review cycles. Audit classification: `temporal`, severity E1/A0/F2/K1/T2 = 6,
confidence C3. The implementation now returns every canonically sorted finding
in one refusal while preserving each stable code/path.

## Counterfactuals

- A — Duplicate semantic checks in native review: direct but guaranteed drift.
  Rejected.
- B — Treat all graph-lint errors/warnings as acceptance blockers: collapses
  deterministic contract and heuristic advice. Rejected.
- C — Call the canonical topology validator on accept, then record heuristic
  lint separately; skip semantic blocking for reject/reopen. Adopted.

## Residual risks and conclusion

- [Hypothesis] A future structured CLI error payload could expose findings as
  an array rather than the current stable code/path text; this is a product
  surface improvement, not an acceptance bypass.
- [Hypothesis] Advisory policy may later need reviewer acknowledgement, but
  promoting heuristic findings to authority now would violate the stated trust
  boundary.

No remaining material local optimum was found in the issue boundary after
aggregating deterministic findings and separating advisories.
