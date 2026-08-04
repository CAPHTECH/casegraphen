# Policies and trust boundaries

Read this when the topology uses verification, budgets, expansion, model
routing, or runtime attestations.

## Verification seam

Model three distinct things:

```text
producer claim -> independently governed verifier -> world anchor
```

Record intended actor/capability separation and required anchors in design
metadata. CaseGraphen can verify ledger identities and capability operations;
runtime session freshness, model/provider identity, and actual context
separation require runtime attestations or external anchors. Never describe
those declarations as verified facts.

Runtime-supplied producer/verifier lineage belongs to the declared lineage
contract and cannot satisfy actor separation, capability, disposition, or
quorum requirements. Only opaque ledger-derived proofs revalidated against the
exact observed ledger enter the strong policy reconciler. Producer and verifier
proofs may come from different historical authority morphisms, but they must
bind the same trace-derived subject revision, claim, topology, node, and
attempt. The strong reconciler also requires the current case space and
rechecks every retained proof; proof construction is not timeless authority.

Do not decide whether a concrete topology's verifier is sufficiently
independent. Supply the policy references and metadata, run `graph lint`, and
preserve its correlation or missing-anchor findings.

## Budget policy

Distinguish design estimates from enforced limits. Name cost, token, latency,
round, spawned-node, and parallelism limits when relevant. Until a runtime
adapter enforces and reports them, they remain proposed deployment policy.

## Expansion policy

Name candidate schema, all-seen dedupe key/scope, dry rounds, maximum
iterations, maximum spawned nodes, and maximum cost. Discovery output is an
unreviewed morphism/topology proposal, not accepted graph state.

## Runtime metadata

Treat reported actor, model, provider, context id, token use, cost, resource
allocation, worktree, and commit as untrusted observations. Preserve them for
lineage and later reconciliation; do not use them alone to satisfy evidence,
review, or independence.
