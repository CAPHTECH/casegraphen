# Execution topology contract

CaseGraphen keeps three contracts separate:

1. The stable **case graph** says what must be achieved, what blocks it, what
   counts as evidence, and what has been accepted.
2. Experimental **execution topology v0** says which runtime nodes perform
   governed work, how typed outputs bind to inputs, which resources collide,
   and where control, evidence, authority, exclusion, and temporal edges exist.
3. The experimental **runtime run graph** reports attempts and artifacts that
   actually occurred. Those reports remain untrusted until the normal evidence
   and review path accepts an associated claim.

An edge is justified when removing it changes permitted execution, acceptance
possibilities, resource safety, or auditability. Therefore every v0 edge carries
a blocking predicate, dependency witness, and removal counterexample. A data
edge additionally binds a declared source output to a declared target input
with one schema. Non-data edges cannot smuggle a data binding.

The Rust parser denies unknown fields and validates node, edge, policy, binding,
and work-cell references. Content hashing normalizes unordered collections so
serialization order is not mistaken for authority. The hash identifies a
proposal; it does not approve or deploy it.
