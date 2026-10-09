# ADR 0086: durable map invocation receipts

Status: proposed source component; runtime acceptance remains unverified.

## Decision

The opt-in runtime map context uses the fixed V2 recipe `runtime.map-context`, revision 1, with the zero-argument `MapSnapshot {}` operation. The runtime admits that closed recipe, writes an intent receipt, performs the existing single bounded map read, and records a versioned digest of the typed `runtime-map-v1` result after the existing owner parser canonicalizes the snapshot against the already-admitted observation and legal-action set. The response identity remains bound to the configured instance, gateway `session_id`, lease, epoch, generation, and fixed correlation; the MCP `mcp_session_id` remains a distinct request identity. Before decision reuse, decision recording, provider reservation, or provider I/O, the durable runtime compares that result digest to the canonical context accepted by the runner and finalizes the receipt.

The receipt binds the current execution lineage and model execution ID, fixed recipe identity, runtime configuration digest, instance, gateway `session_id`, separate MCP `mcp_session_id`, lease and epoch, state and generation, map profile and schema digest, fixed RPC correlation, request digest, typed result digest, owner snapshot digest, and effective map-aware decision fingerprint. Current nonsecret runtime identity metadata comes from the admitted `RuntimeConfig` retained by the owner-local `DurableHandle`; it is not reconstructed from stored receipt rows. SQLite constrains the fixed recipe, profile, correlation, digest lengths and state-dependent field shape. The store reader validates the exact schema identity and digest syntax, while transactional store updates enforce monotonic state transitions. The store caps retained receipt rows at 4,096 and refuses a new intent at capacity; it does not evict or expire rows.

## Recovery and data limits

The runtime port reports `map_snapshot_invalid` only when the existing canonical snapshot parser refuses the owner context after the typed response envelope and invocation identity pass validation. Read, envelope, identity, and receipt persistence failures remain `map_snapshot_failed`. Both errors are nonretryable. Snapshot refusal retains the intent without a validated response digest and blocks another read for that invocation; it does not reserve provider budget. Synthetic fixtures test this ordering and classification but do not establish authoritative owner provenance.

Receipts contain identity metadata and digests only. They do not store the map, protocol text, prompts, provider payloads, credentials, or raw errors. Intent does not prove that a request was sent. A validated response does not prove owner acceptance. Owner finalization does not prove provider dispatch. A digest cannot reconstruct a map context; the receipt records local processing history and is never a cross-invocation cache.

Here, an owner-accepted context means the current runtime parser accepted the supplied snapshot against its admitted observation, legal-action set, and configured invocation identity. Those checks establish consistency between supplied facts and the runtime's current binding. The receipt and its digests do not independently attest that an authoritative native owner produced those facts. Authentic owner provenance requires its separate transport and native acceptance evidence.

An existing receipt blocks a repeated map read for the same invocation. Intent or response receipts that were not finalized in the current process remain blocked after restart and cannot reconstruct or replay a snapshot. A completed `ContextValidated` receipt may support reuse only when the exact current binding, owner-accepted canonical snapshot, and effective decision fingerprint match. Existing interrupted-unknown and reconstruction-required recovery rules remain authoritative. The collector's existing raw 256 KiB and escaped outer 512 KiB limits remain unchanged; this hash-only table adds no payload limit.

## Migration and verification boundary

Schema version 9 adds the receipt table. Migration checks the actual SQLite version after the worker v7 migration because that migration advances the database to v8 while an earlier local version variable can remain stale. Version 8 migrates transactionally to v9, version 9 is stable, and newer versions are rejected.

This decision records the source contract for one bounded component. It does not claim native game execution, provider readiness, deployment, issue completion, or successful served-runtime recovery. The source worker did not run checks; the PR records supervisor validation separately. Non-map decisions preserve the existing approved-reference contract when neither a pending invocation nor a durable receipt exists for that execution. A durable receipt still blocks omission of its map context after restart.
