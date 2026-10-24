# ADR 0086: durable map invocation receipts

Status: proposed source component; runtime acceptance remains unverified.

## Decision

The opt-in runtime map context uses the fixed V2 recipe `runtime.map-context`, revision 1, with the zero-argument `MapSnapshot {}` operation. The runtime admits that closed recipe, writes an intent receipt, performs the existing single bounded map read, and records a versioned digest of the typed `runtime-map-v1` result after the existing owner parser canonicalizes the snapshot against the already-admitted observation and legal-action set. The response identity remains bound to the configured instance, gateway `session_id`, lease, epoch, generation, and fixed correlation; the MCP `mcp_session_id` remains a distinct request identity. Before decision reuse, decision recording, provider reservation, or provider I/O, the durable runtime compares that result digest to the canonical context accepted by the runner and finalizes the receipt.

The receipt binds the current execution lineage and model execution ID, fixed recipe identity, runtime configuration digest, instance, gateway `session_id`, separate MCP `mcp_session_id`, lease and epoch, state and generation, map profile and schema digest, fixed RPC correlation, request digest, typed result digest, owner snapshot digest, and effective map-aware decision fingerprint. Current nonsecret runtime identity metadata comes from the admitted `RuntimeConfig` retained by the owner-local `DurableHandle`; it is not reconstructed from stored receipt rows. The schema enforces the fixed profile, schema identity, correlation, and monotonic receipt state. The store caps retained receipt rows at 4,096 and refuses a new intent at capacity; it does not evict or expire rows.

## Recovery and data limits

Receipts contain identity metadata and digests only. They do not store the map, protocol text, prompts, provider payloads, credentials, or raw errors. Intent does not prove that a request was sent. A validated response does not prove owner acceptance. Owner finalization does not prove provider dispatch. A digest cannot reconstruct a map context, so a receipt is provenance and never a cross-invocation cache.

An existing receipt blocks a repeated map read for the same invocation. Intent or response receipts that were not finalized in the current process remain blocked after restart and cannot reconstruct or replay a snapshot. A completed `ContextValidated` receipt may support reuse only when the exact current binding, owner-accepted canonical snapshot, and effective decision fingerprint match. Existing interrupted-unknown and reconstruction-required recovery rules remain authoritative. The collector's existing raw 256 KiB and escaped outer 512 KiB limits remain unchanged; this hash-only table adds no payload limit.

## Migration and verification boundary

Schema version 9 adds the receipt table. Migration checks the actual SQLite version after the worker v7 migration because that migration advances the database to v8 while an earlier local version variable can remain stale. Version 8 migrates transactionally to v9, version 9 is stable, and newer versions are rejected.

This decision records the source contract for one bounded component. It does not claim native game execution, provider readiness, deployment, issue completion, or successful runtime recovery. Builds, format checks, and tests were not run by the source worker and remain for root-owned review and validation.
