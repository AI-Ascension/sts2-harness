# ADR 0003: Harness Initial Ports and Deterministic Seams

## Status

Accepted for Wave 2 codebase initialization. This decision authorizes preparation seams only; it
does not authorize live integrations or a released runtime API.

## Context

The harness must coordinate independent runs and episodes while preserving instance, session,
request, model-execution, trajectory, record, and artifact identity. It needs testable boundaries
for routing, model/provider execution, storage, replay, and artifact publication without contacting a
game, gateway, MCP server, provider, or external store. Retry and cleanup behavior must be observable
without relying on timing or network behavior.

## Decision

The target-owned `crates/harness` package provides a standard-library-only coordinator and explicit
ports:

| Port or seam | Contract owned by the harness |
|---|---|
| `InstanceRouter` | Bind a run/episode to an instance and gateway-session reference; unbind it during close. The gateway still owns leases, routing, and fencing. |
| `ProviderPort` | Execute a provider-neutral model request with stable correlation and idempotency across bounded retries. Provider SDKs and credentials stay outside. |
| `RecordPort` | Append and read bounded, correlated trajectory records with sequence and idempotency identity scoped to a trajectory. Storage implementation stays outside. |
| `ReplayPort` | Consume a trajectory and return a deterministic replay report or divergence metadata. It does not simulate game rules. |
| `ArtifactPort` | Publish bounded bytes with owner run, schema version, content digest, producer, and lineage metadata. The artifact store stays outside. |

The coordinator allocates distinct nonzero identities for runs, episodes, trajectories, instances,
gateway sessions, requests, model executions, traces, records, and artifacts. An episode keeps its
route binding and base correlation together. A record retry with the same idempotency key returns the
original record only when its kind and payload match; conflicting content returns the stable
`record_idempotency_conflict` storage error without appending or advancing sequence. Artifact drafts
verify the lowercase SHA-256 digest against their exact bytes before publication; a mismatch returns
`artifact_digest_mismatch`. These are safety corrections to the unreleased preparation API, not a
new persistent-store or cross-repository wire contract.
A model retry reuses the same request, correlation, execution identity, and
idempotency key. Shutdown unbinds every active episode, closes every port, reports each failure, and
is idempotent.

The package has no cross-repository path dependencies. Core code contains no transport, host,
process, MCP framing, provider implementation, gateway lease authority, credential handling, or game
rule. `sts2-protocol` remains the accepted sixth implementation target for narrow shared,
language-/transport-neutral contracts; this package does not create a generic-common owner or
duplicate protocol implementation internals.

## Evidence and conformance

Target-local integration tests use deterministic fake router, provider, storage, artifact, and replay
ports. They cover route correlation, transient retry and idempotency reuse, append deduplication,
trajectory replay, artifact metadata/lineage, and one-time close/cleanup. These tests are offline
source evidence for the seam behavior only. They do not establish gateway, MCP, provider, artifact
store, host, or game compatibility.

Future serialized contracts must name their producer and consumers, record independent harness,
protocol/profile, schema, provider, gateway/MCP, game/mod, evaluator, and runtime versions, and add
versioned conformance vectors before they are treated as compatibility guarantees.

## Revisit conditions

Revisit this decision before adding a live adapter, external dependency, serialized public format,
scoring policy, dataset exporter, training integration, or any authority over game state, leases, or
MCP framing. The revision must state ownership, dependency direction, bounded failure behavior,
provenance, and deterministic acceptance evidence.
