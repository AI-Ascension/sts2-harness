# Typed recipe contract v2

**Status:** Accepted for the additive contract and admission slice.

**Context:** [Issue #97](https://github.com/AI-Ascension/sts2-harness/issues/97)
requests typed recipe arguments, a fixed mapping boundary, and durable
pre-context collection. Recipe V1 provides effect-free admission over a fixed catalog, but
its authored arguments are strings and its admitted value does not retain a
typed result domain. The runtime map protocol already defines a closed,
bounded typed envelope and snapshot. The Harness also has an owner-side map
validator that enforces current map semantics and legal-action binding.

**Decision:** Add a separate recipe V2 contract with one fixed operation,
`map_snapshot`, and no caller-supplied arguments. The wire version is the
closed `ascension.recipe/v2` enum value. Admission validates the existing
bounded `RecipeId` form and requires a nonzero `RecipeRevision`. The result
domain is `sts2_protocol::RuntimeMapV1Snapshot`; no generic JSON value,
schema-name string, arbitrary tool, URL, endpoint, or interpolation appears in
the V2 contract.

Preserve all V1 recipe and workflow source/API behavior. This slice adds only
the contract and pure admission. It does not wire a catalog mapping, runtime
read, collection receipt, workflow binding, store, Studio shape, or production
execution path. It makes no production or native-runtime capability claim.

Any later collector must decode the complete map envelope with the pinned
`sts2_protocol::decode_runtime_map_message`, retain its correlation,
provenance, instance/session/lease, epoch, and generation, then reuse the
existing Harness `MapDecisionContext` validator for current map and
legal-action semantics. The contract slice adds no duplicate map validator.
Observation and legal actions already admitted for a decision remain the
inputs to a later collector; they are not fetched again.

The first collector remains separately bounded to one fixed read, 64 KiB per
result and complete receipt, a 5-second read timeout, a 10-second total
deadline, and no cross-invocation cache. Oversize data is rejected without
truncation. These limits do not imply that collection is implemented here.
Existing encrypted store patterns bound owner receipts to 64 KiB plaintext
and cap immutable source/publication rows at 16 per run, but those rows do not
expire automatically. A later collection store must have explicit bounded
rows and run-store lifetime retention; this decision adds no store change.

**Consequences:** Existing V1 definitions remain accepted under their existing
rules. V2 authors can express only the one typed read at the contract layer,
but no current workflow or production component invokes it. Coordinated
workflow V2 producer/consumer support and the durable pre-inference collector
remain later work before authored bindings can be enabled.
