# ADR 0076: Scoped research inspection of hidden checkpoint state

- Status: Accepted
- Date: 2026-09-24
- Issue: [#129](https://github.com/AI-Ascension/sts2-harness/issues/129)
- Extends: [ADR 0016](0016-checkpoint-integrity-admission.md),
  [ADR 0037](0037-provider-session-boundary.md)

## Context

The harness deliberately keeps several classes of state away from ordinary lanes.
`RuntimeV3` gameplay observations exclude RNG streams, unrevealed outcomes and
private host state, and public checkpoint references are digest-free by
construction: a caller holds a keyed opaque handle, never an exact private hash.
That boundary is a product property, not an accident, and it must not weaken.

It is not the whole requirement. Reproducing a divergence, auditing a capture or
explaining a branch outcome sometimes needs the hidden facts a capture recorded:
the exact RNG cursors, the stored pile order, the assignments the client had not
revealed yet. Refusing that work outright pushes it into ad-hoc local analysis
outside any reviewed boundary, which is worse for both fairness and auditability.

The harness therefore needs a *second* permission. The decision below records how
that permission is expressed so that the ordinary boundary is preserved verbatim
and the new one is reachable only through an explicit, bounded approval.

## Decision

We recognise four distinct visibility classes and keep them separate:

1. **Public live facts** — what an ordinary gameplay lane may observe about a run.
   Unchanged by this record.
2. **Static reference knowledge** — catalog and rule data that carries no run
   identity. Unchanged by this record.
3. **Player-discovered knowledge** — facts a run has legitimately revealed.
   Unchanged by this record.
4. **Privileged research data** — hidden checkpoint state. Reachable only through
   `harness::research_inspection`, and only under an operator-admitted grant.

The research permission is a **new feature**, not a reinterpretation of the
existing fair-play grant. Nothing in this record widens an ordinary lane, changes
a public projection, or makes an existing digest-free handle reveal a digest.

### Operator-supplied scope

Scope lives in the grant, never in the request. `ResearchInspectionGrant::admit`
takes the exact checkpoint, run and branch, one consumer lane, and an explicit,
non-empty set of field groups no larger than `MAX_RESEARCH_GRANTS`. Because the
candidate visibility is fixed at approval time, a gameplay lane cannot escalate
by asking for a different visibility parameter: naming a group outside the grant
is `ProtectedField`, a different checkpoint/run/branch is `WrongTarget`, and a
different lane or a revoked grant is `RevokedScope`.

Revocation is monotonic. `revoke` takes `&mut self`, so re-admitting the same
identity cannot resurrect the approval, and a cached or replayed request cannot
outlive it.

### A closed field matrix

A research read names field *groups* from a closed vocabulary — RNG streams,
hidden piles, unrevealed assignments, pending hidden state and save-state
fields — plus a bounded identifier inside the group. There is no object,
reflection, memory or file query: a field reference refuses `/`, `\`, whitespace,
`..`, a leading or trailing `.`, uppercase and any value over
`MAX_FIELD_REF_BYTES`, so an arbitrary path or query expression is not
expressible. Adding a group is additive and never reinterprets an existing label.

### Admission is not an answer

The harness owns no native capture reader, so it must never fabricate a field's
availability. `admit_research_read` therefore returns an `AdmittedResearchRead`
naming exactly which fields an owner may report, on which page, and whether that
page is the whole answer. The owner holding the verified capture answers with a
`ResearchPageReport`, which `verify` checks back against the admission
structurally: the page index must match, and the entries must be exactly the
admitted fields, once each, in the admitted order. A report that adds, omits,
reorders or mislabels a field is refused, so a value can only ever come from the
owner that holds the capture.

Coverage is reported per field and is never collapsed into a value. A field the
capture did not store is `NotMaterialized`, not zero and not an empty string; a
field whose value depends on a future outcome is `SimulationRequired` with its
dependency named, and this contract never simulates it; a field the native owner
does not expose is `Unsupported`. Only `Available` carries a value. Completeness
is likewise explicit: page zero of a multi-page answer is never labelled
complete.

### Bounds, refusals and read-only behaviour

Reads are paged and bounded by `MAX_RESEARCH_PAGE_ITEMS`, and an out-of-range
page or page size is `InvalidPage` rather than a silent truncation that looks
complete. The refusal vocabulary is closed and value-free: no variant, and no
`Display` text, carries a field name, a value or a digest, so a lane cannot learn
hidden data by making a request fail. Nothing in the module reads, simulates,
restores or mutates any checkpoint or game state.

## Consequences

- The ordinary boundary is preserved unchanged: public handles stay digest-free
  and no default provider context includes hidden fields.
- Hidden state becomes reachable only through an auditable, revocable,
  single-lane approval bound to one exact checkpoint.
- The native read adapter and the capture-coverage manifest agreement stay with
  the native owner (`sts2-game-mod`); this record fixes the source-only contract
  they answer through and does not claim a native effect.
- Because admission returns no values, a future owner adapter cannot pass a
  synthesized or default value off as a captured one.

## Alternatives considered

- **Widen the fair-play grant.** Rejected: it would make the ordinary boundary
  conditional on operator policy, so a misconfiguration would silently expose
  hidden state to a gameplay lane instead of being refused.
- **Let the request carry its own visibility parameter.** Rejected: the request is
  authored by the lane being constrained, so this is privilege escalation by
  construction.
- **Return a value for every field, using a sentinel for missing data.**
  Rejected: a sentinel is indistinguishable from a real captured value, and the
  issue explicitly forbids converting unknown or unsupported into zero, empty or
  an invented description.
