<!-- SPDX-License-Identifier: MIT -->
# ADR 0081: Harness seed-support catalog v2

## Status

Accepted for the Harness management source/component contract. Native target setup, seed settlement, deployment, and release acceptance remain unverified.

## Context

The versioned V2 workflow request supports explicit and derive-once seed modes, but discovery previously exposed only the actor-scoped V1 target catalog. Clients need a query-free way to inspect which candidate-binding modes the current Harness store can durably support, while keeping storage capability distinct from current target availability, key readiness, and launch setup.

ADR 0003 requires a new decision when adding a serialized public format. This record covers the Harness management route and does not add a neutral cross-repository protocol artifact.

## Decision

The Harness management service is the producer of a closed `ascension.workflow-targets/v2` response from query-free `GET /v2/workflow-targets`. Authenticated workflow-management clients with `workflow:read` may consume it. Each target embeds the exact actor-scoped V1 descriptor and its digest. The catalog copies the V1 `catalog_revision` and carries the independent `support_revision: "seed-support.v1"`.

The response separates `durable_candidate_binding_modes` from the non-authoritative `ready_candidate_modes` hint. Durable explicit support requires durable seed-binding storage. Durable derive-once support also requires durable operation reservations and a configured key authority. An authority that is configured but reports `Unknown` can remain structurally advertised. The ready list is empty for an unavailable target; derive-once is ready only when the target is available and the authority reports `Ready`. A client must not treat readiness as proof that current-key retrieval or submission will succeed. `supported_launch_setups` remains empty because this schema defines no launch-setup values.

Derive-once first requires a store that supports operation-reservation lookup; unsupported stores fail closed before lookup because they cannot inspect or recover persisted operations. On a supported store, the additional structural mode requirement applies only after the initial operation or binding lookup returns `Missing`. The service first revalidates current owner target admission, then checks durable mode support before inference-profile admission, current-key retrieval, reservation or binding persistence, and execution. The check does not supersede historical recovery: `CandidatePersisted` uses the existing exact submission path, and `Prepared` derive-once recovery uses stored admission and the pinned historical key version. Readiness is not a key-rotation mechanism; the file authority is an immutable process-start snapshot.

## Compatibility and migration

This is additive for Harness management clients. `GET /v1/workflow-targets` and the V1 response remain unchanged; the `POST /v2/workflow-runs` request shape is unchanged. Existing clients may continue using V1. Clients that need seed-support discovery can opt into the authenticated V2 route and must preserve the distinction between structural support, readiness hints, and launch setup. Unsupported new submissions fail closed at the durable-mode gate after owner admission revalidation.

No `sts2-protocol` artifact or cross-repository schema is introduced. The catalog is owned by the Harness management service.

## Evidence and limits

At signed source commit `384c6d5878a6c10bb8ffb87ada54870eb4a0abea`, root-qualified format, strict policy, Clippy, strict-docs, workspace tests, and doctests passed. The direct-dispatch route tests and local SQLite policy tests provide source/component evidence for the response and Missing-path boundary. They do not establish a bound HTTP listener, target launch, native setup or seed settlement, deployment, or release compatibility.
