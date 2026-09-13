# Effective context limits

The harness owns the executable limits for context memory and provider sessions. JSON Schema
ceilings are portable syntax ceilings only; they do not authorize execution. A caller must validate
the closed, versioned capability descriptor and bind it to the selected policy/profile before
inference, retention, or provider admission.

## Producer inventory

| Public field | Owner / validator | Classification | Schema ceiling | Harness effective ceiling |
| --- | --- | --- | ---: | ---: |
| `max_candidates` | `MemoryPolicy::validate_schema` / `validate_against_capabilities` | policy/profile | 64 | 64 |
| `max_results` | same | policy/profile | 16 | 16 |
| `max_selected` | same | policy/profile | 32 | 32 |
| `optional_byte_budget` | same | policy/profile | 65,536 bytes | 8,192 bytes |
| `max_entries_per_run` | `MemoryCorpus::with_limits` | corpus/profile | 10,000 | selected corpus value |
| `max_corpus_bytes` | `MemoryCorpus::with_limits` | corpus/profile | 256 MiB | selected corpus value |
| `max_source_bytes` | memory admission | global schema/runtime | 64 KiB | 64 KiB |
| `max_sources_per_job` | summary job admission | global schema/runtime | 16 | 16 |
| `max_job_input_bytes` | budget/selection admission | global schema/runtime | 64 KiB | 64 KiB |
| `max_summary_output_bytes` | summary output admission | global schema/runtime | 8 KiB | 8 KiB |
| `max_query_bytes` | query validator | global schema/runtime | 4 KiB | 4 KiB |
| `max_lineage_depth` | provenance validator | global schema/runtime | 2 | 2 |
| `max_global_memory_bytes` / `max_global_memory_jobs` | `MemoryBudgetLedger` | global runtime | 256 KiB / 32 | same |
| `max_retention_resources` / `max_retention_bytes` | `RetentionInventory` | retention runtime | 512 / 256 MiB | same |
| `max_cache_entries` | `RetrievalCache` | projection runtime | 256 | same |
| `max_review_records` | `ImmutableReviewLedger` | review runtime | 512 | same |
| `max_memory_bindings` | `AtomicBindingStore` | binding runtime | 256 | same |
| `max_usage_attempts` | `UsageLedger` | telemetry runtime | 1,024 | same |

Provider-session descriptors publish the corresponding `MAX_*` values in
`contracts/provider-session/capabilities.schema.json`. The transport implementation also has
adapter-private parser bounds (native state bytes/entries/depth, outstanding requests, argument
and environment-name counts); these are implementation guards and are deliberately not inferred
as model or policy capacity.

## Capability binding

Context-memory capabilities use
`ascension.context-memory.capabilities.v3`; provider capabilities use
`ascension.provider-session.capabilities.v3`. Each descriptor contains an owner, owner revision,
exact policy-schema digest, model/adapter revision and a descriptor SHA-256 over the complete
payload with that digest field cleared. Unknown, disabled, unattached, and descriptors stale
relative to trusted pins are not treated as unlimited. A changed limit, profile, adapter revision,
or copied schema fails descriptor validation.

The digest is an integrity marker, not a signing key. A consumer that receives a descriptor across
an owner boundary must additionally pin the expected owner/model/adapter revisions with
`validate_against_trusted`; those pins must come from trusted configuration, never from the
descriptor itself. The producer must update the schema, revision, and digest together. Consumers
should forward the descriptor unchanged and validate the digest before using any effective value.
A schema version change is additive only when a consumer has explicitly negotiated that version;
otherwise the older descriptor is rejected.

## Boundary matrix

For each bounded value, evaluate the portable schema and selected profile independently:

| Input | Portable schema result | Selected profile result | Studio/selection result |
| --- | --- | --- | --- |
| lower than both ceilings | valid | admissible | available |
| exactly at schema and effective ceiling | valid | admissible | available |
| one above effective but at/below schema ceiling | valid | rejected with an effective-limit error before inference/retention | unavailable; retain for inspection |
| one above the schema ceiling | invalid | not evaluated | unavailable |
| descriptor missing, malformed, stale relative to trusted pins, or wrong profile/adapter | descriptor invalid | rejected before admission | unavailable; absent is not unlimited |

## Explicit policy migration

A schema-valid policy above the selected effective limit remains immutable and inspectable. The
store-facing `PolicyMigrationProposal::new_from_bytes` constructor records the exact original
serialized bytes, their digest, the target capability digest, and each requested/effective
mismatch. Creating a proposal does not clamp values, activate a policy, or alter history. An
operator must call `approve` and then provide an independently authored, bounded target policy to
`adopt`; adoption requires a new policy version and leaves the original bytes and audit record
intact.
