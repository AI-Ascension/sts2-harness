# Effective context limits

The harness owns the executable limits for context memory and provider sessions. JSON Schema
ceilings are portable syntax ceilings only; they do not authorize execution. A caller must validate
the closed, versioned capability descriptor and bind it to the selected policy/profile before
inference, retention, or provider admission.

## Machine-readable classification record

Each surface publishes an `ascension.harness.effective-limits.v1` record
([schema](../contracts/effective-limits.schema.json)) beside its capability descriptor. The record
classifies every advertised value and states the executable ceiling for the selected owner/profile:

| Class | Meaning |
| --- | --- |
| `schema_executable_equal` | Portable policy-schema ceiling and executable ceiling are identical. |
| `schema_broader_than_executable` | The portable schema intentionally admits more than this profile executes. |
| `profile_selected` | The ceiling is selected per corpus/profile; the portable policy schema has no such field. |
| `runtime_guard` | Runtime or transport guard with no portable policy field. |

Row field names match the matching capability descriptor's `effective_limits` object, so a consumer
can compare the record with the descriptor directly. `policy_schema_ceiling` is present only when a
portable policy field exists; `capabilities_schema_ceiling` is the largest value the capability
schema lets the descriptor publish; `executable_ceiling` is authoritative.

Producers: `MemoryCapabilities::effective_limit_record()` and
`NativeCapabilities::effective_limit_record()`. The compiled `context-memory-cli limits` command
prints the record as JSON. Consumers must admit a value before presenting it:

- `EffectiveLimitRecord::admit(field, requested)` answers selected-profile admissibility only.
- `EffectiveLimitRecord::admit_authorized(trusted, field, requested)` authenticates the complete
  record against the record derived from the validated trusted capability descriptor (never from the
  record under test), then admits the value. A record that keeps a trusted identity label but
  changes any limit, class, ceiling, owner, or `enabled` flag fails closed.

Unavailable reasons are machine-readable: `effective_limit_exceeded`, `disabled`,
`field_not_advertised`, `descriptor_stale`, `descriptor_tampered`, `profile_mismatch`,
`consumer_not_recorded`, and `consumer_pin_not_adopted`. A field absent from the record is
unavailable, never unlimited.

## Producer inventory

| Public field | Owner / validator | Class | Portable ceiling | Executable ceiling |
| --- | --- | --- | ---: | ---: |
| `max_candidates` | `MemoryPolicy::validate_schema` / `validate_against_capabilities` | `schema_executable_equal` | 64 | 64 |
| `max_results` | same | `schema_executable_equal` | 16 | 16 |
| `max_selected` | same | `schema_executable_equal` | 32 | 32 |
| `optional_byte_budget` | same | `schema_broader_than_executable` | 65,536 bytes | 8,192 bytes |
| `max_entries_per_run` | `MemoryCorpus::with_limits` | `profile_selected` | 10,000 | selected corpus value |
| `max_corpus_bytes` | `MemoryCorpus::with_limits` | `profile_selected` | 256 MiB | selected corpus value |
| `max_source_bytes` | memory admission | `runtime_guard` | — | 64 KiB |
| `max_sources_per_job` | summary job admission | `runtime_guard` | — | 16 |
| `max_job_input_bytes` | budget/selection admission | `runtime_guard` | — | 64 KiB |
| `max_summary_output_bytes` | summary output admission | `runtime_guard` | — | 8 KiB |
| `max_query_bytes` | query validator | `runtime_guard` | — | 4 KiB |
| `max_lineage_depth` | provenance validator | `runtime_guard` | — | 2 |
| `max_global_memory_bytes` / `max_global_memory_jobs` | `MemoryBudgetLedger` | `runtime_guard` | — | 256 KiB / 32 |
| `max_retention_resources` / `max_retention_bytes` | `RetentionInventory` | `runtime_guard` | — | 512 / 256 MiB |
| `max_cache_entries` | `RetrievalCache` | `runtime_guard` | — | 256 |
| `max_review_records` | `ImmutableReviewLedger` | `runtime_guard` | — | 512 |
| `max_memory_bindings` | `AtomicBindingStore` | `runtime_guard` | — | 256 |
| `max_usage_attempts` | `UsageLedger` | `runtime_guard` | — | 1,024 |
| `max_completed_turns` | `ProviderSessionPolicy::validate_schema` / `ProviderSessionBroker::new` | `schema_broader_than_executable` | 1,024 | 128 |
| `max_history_ttl_seconds` | same | `schema_broader_than_executable` | 604,800 s | 86,400 s |

Provider-session descriptors publish the corresponding `MAX_*` values in
`contracts/provider-session/capabilities.schema.json`. The transport implementation also has
adapter-private parser bounds (native state bytes/entries/depth, outstanding requests, argument and
environment-name counts); these are implementation guards and are deliberately not inferred as
model or policy capacity.

## Capability binding

Context-memory capabilities use `ascension.context-memory.capabilities.v3`; provider capabilities
use `ascension.provider-session.capabilities.v3`. Each descriptor contains an owner, owner revision,
exact policy-schema digest, model/adapter revision and a descriptor SHA-256 over the complete payload
with that digest field cleared. Unknown, disabled, unattached, and descriptors stale relative to
trusted pins are not treated as unlimited. A changed limit, profile, adapter revision, or copied
schema fails descriptor validation.

The digest is an integrity marker, not a signing key. A consumer that receives a descriptor across
an owner boundary must additionally pin the expected owner/model/adapter revisions with
`validate_against_trusted`; those pins must come from trusted configuration, never from the
descriptor itself. The producer must update the schema, revision, and digest together. Consumers
should forward the descriptor unchanged and validate the digest before using any effective value.
A schema version change is additive only when a consumer has explicitly negotiated that version;
otherwise the older descriptor is rejected.

## Producer/consumer pin and digest conformance matrix

[`contracts/effective-limits-pins.json`](../contracts/effective-limits-pins.json) records the
producer contract digests, the recorded consumer pins, and each consumer's adoption state.
`PinMatrix::validate()` recomputes the producer digests from repository bytes and rejects:

- producer artifact drift, including a missing or unknown contract path;
- a consumer revision that is not a full 40-hex revision;
- a harness CI pin whose workflow ref disagrees with the recorded consumer pin;
- an `aligned` consumer whose declared capability schema, effective-limit disclosure, or copied
  contract digests do not match the producer;
- a `pending` consumer whose entry already matches the producer pins (a stale label).

Producer digests are `confirmed` offline because they are recomputed from repository bytes. Recorded
consumer digests are `source-derived` at the listed revision and are re-verified by that consumer's
own pinned lane; this check detects harness-side drift, not a consumer that moved without recording it.

Both recorded consumers are `pending`, because neither forwards the `v3` effective-limit record yet:
`ascension-context-console` copies `ascension.*.capabilities.v1` schemas without `effective_limits`,
and `ascension-workflow-studio` validates `ascension.context-memory.capabilities.v1` without
disclosing effective limits. `PinMatrix::admit_consumer(...)` authenticates the record against the
trusted derivation before admitting it, so a copied-contract consumer must record the complete
producer artifact inventory; both recorded consumers fail closed with `consumer_pin_not_adopted`, so
neither can present a value the runtime rejects. A consumer that declares an unrecorded surface also
fails closed rather than defaulting to unlimited.

## Boundary matrix

For each bounded value, evaluate the portable schema, the selected profile, and the consumer entry
independently:

| Input | Portable schema result | Selected profile result | Consumer/Studio result |
| --- | --- | --- | --- |
| lower than both ceilings | valid | admissible | available when the consumer pin is aligned |
| exactly at the effective ceiling | valid | admissible | available when the consumer pin is aligned |
| one above effective but at/below schema ceiling | valid | rejected with `effective_limit_exceeded` before inference/retention | unavailable; retain for inspection |
| one above the schema ceiling | invalid | not evaluated | unavailable |
| descriptor missing, malformed, stale relative to trusted pins, or wrong profile/adapter | descriptor invalid | rejected before admission | unavailable; absent is not unlimited |
| consumer pin pending, absent, or unrecorded | unchanged | unchanged | unavailable; `consumer_pin_not_adopted` or `field_not_advertised` |

`crates/harness/tests/effective_limits_conformance.rs` checks the lower, exact, and one-over
boundary for every published value against schema validity and profile admission separately, and
asserts that the published ceilings equal both the policy/capability schema maxima and the runtime
constants. `crates/harness/tests/effective_limits_consumer_pins.rs` adds the consumer-side guard and
the fail-closed pin/tamper cases.

## Explicit policy migration

A schema-valid policy above the selected effective limit remains immutable and inspectable. The
store-facing `PolicyMigrationProposal::new_from_bytes` constructor records the exact original
serialized bytes, their digest, the target capability digest, and each requested/effective mismatch.
Creating a proposal does not clamp values, activate a policy, or alter history. An operator must call
`approve` and then provide an independently authored, bounded target policy to `adopt`; adoption
requires a new policy version and leaves the original bytes and audit record intact.

## Remaining external gates

These checks are deterministic and offline. They do not install or build a consumer, exercise a
browser, launch a game, or contact a provider. Consumer adoption requires the consumer repository to
copy the `v3` record, update its pin/digest entry to `aligned`, and pass its own pinned-lane
verification; native/provider/deployment acceptance remains separately authorized and unverified.
