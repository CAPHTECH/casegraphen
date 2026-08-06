# Memory Plane threat model

## Protected assets

- accepted Case Graph state and review authority;
- source bytes and provenance roles;
- current temporal meaning of project constraints and decisions;
- actor/scope/sensitivity isolation;
- reproducibility of projections and derived indexes;
- agent action proposals that consume memory context.

## Trust boundaries

External sources, tool output, runtimes, LLM extraction, MCP callers, Memory Use
Reports, summaries, embeddings, and indexes are untrusted. The replayed
MorphismLog, exact artifact bytes, canonical operation gates, and accepted
review records are authoritative only for the procedural facts they establish.
They do not prove an accepted proposition is absolute truth.

## Threats and controls

| Threat | Attack | v0 control | Retained observation |
|---|---|---|---|
| Provenance laundering | External text is summarized as a project policy | Source/provenance roles remain typed; dual authority ceilings; accepted hard `authorized_by` required for elevation | `authority_amplification` finding |
| Caller-declared trust | MCP payload sets `accepted` or high trust | Strict contracts reject unknown acceptance fields; proposal output is always unreviewed/non-mutating | negative parse and proposal tests |
| Source substitution | Claim cites different bytes after extraction | `sha256:` Source Record hash and exact `artifact:sha256-...` relation | hash mismatch fixture |
| Stale memory | Expired or superseded decision ranks highly | valid-time/status filters before ranking; normal query excludes both | bitemporal tests |
| Hidden conflict | Two incompatible accepted constraints are silently ranked | accepted contradictions derive contested state; hard conflicts excluded and listed | conflict fixture |
| Scope confusion | Actor A preference is reused for actor B/project C | actor grant, project/case/actor scope, sensitivity, audience, and purpose filters | cross-actor fixture |
| Repetition authority | Many low-authority sources imitate consensus | authority is a ceiling, not a score; repetition changes neither ceiling nor status | poisoning corpus |
| Conditionality loss | A condition-specific outcome becomes a global rule | structured scope/valid time; unsupported generalization is a validation/audit finding | conditional-decision fixture |
| Index poisoning | Vector/lexical index inserts an invisible claim | index is built only from filtered projection, content-addressed, non-authoritative, and rebuild-validated | rebuild-equivalence test |
| Transaction-time confusion | Caller queries a current snapshot using an old revision label | exact `base_revision_id` equality; older history requires replay of that revision | stale-revision rejection |
| Memory-action gap | Agent receives a constraint but ignores it | projection content hash binds action context; Use Report is retained as untrusted observation | projection/use-report validation |
| Review capture | Proposer approves its own inference | no Memory Plane acceptance API; existing reviewer independence and operation gate remain in force | existing review-gate suite |

## Security invariants

The release gate requires zero accepted claims without sources, zero silent
authority amplification, zero caller-declared acceptance, zero non-replayable
accepted memory, zero hidden hard conflict, and zero expired claim returned as
current. A failure is a release blocker, not a ranking-quality degradation.

## Residual risks

- Source-backed and reviewed propositions can still be factually wrong. The
  guarantee is procedural acceptance with inspectable evidence and authority.
- A malicious authorized reviewer can elevate a false claim; reviewer identity
  and binding are auditable but human judgment is not automated.
- Token estimates are deterministic approximations, not model-specific counts.
- v0 does not solve personal-data erasure in an append-only log. Project memory
  only is permitted until cryptographic erasure, tombstones, artifact separation,
  and derived-index purge receive a separate ADR.
- A runtime may lie in its Use Report. Action traces and deterministic guards
  are needed to prove use rather than merely record a claim of use.
