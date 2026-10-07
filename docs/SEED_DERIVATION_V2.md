<!-- SPDX-License-Identifier: MIT -->
# Derive-once seed operations (v2)

This guide documents the additive, versioned derive_once mode for Harness issue [#103](https://github.com/AI-Ascension/sts2-harness/issues/103). It records the approved restart-stable derivation contract; it does not change the frozen seeded-run-v1 request or transport.

## Behavior and compatibility

The v2 request is closed and actor-bound. An explicit seed carries the caller's canonical seed. A derive_once request carries no seed or key-selection field. It replaces the older “draw exactly once” description for this v2 mode: the effective seed is deterministic HMAC output keyed by the durable operation identity, not a new random draw on each attempt.

For a new derive-once operation, the service validates the current owner admission and computes the request, actor, run, operation, and configuration identities. It then atomically reserves a key_pinned operation in SQLite before deriving a seed. The reservation binds the actor/request/run/operation and admitted configuration to algorithm hmac-sha256-v1 and an immutable key authority/version/commitment. Concurrent contenders use the stored winner; a contender's provisional current key does not replace or conflict with that winner. The service reloads the winner's exact historical key version, verifies its full commitment, and derives from a length-prefixed, domain-separated transcript containing the algorithm, authority/version, actor subject, request digest, workflow run ID, operation ID, and configuration digest. HMAC-SHA-256's first 16 bytes become 32 lowercase hexadecimal seed characters.

The derived candidate and its initial run/submission records are committed atomically. A strong durable readback precedes any execution effect. Retries and restarts read the persisted operation and candidate; they do not select the new current key or derive from changed configuration. Missing historical versions, changed key material, corrupt indexes, actor/request conflicts, or unavailable durable stores fail closed. A failed candidate transaction may leave a key-pinned reservation; retry resumes that reservation with the pinned version.

V1 and v2 remain distinct. The existing SeedMode::GenerateOnce behavior, canonical explicit seed handling, and seeded-run-v1 handoff remain governed by [ADR 0071](decisions/0071-authored-seed-binding.md) and [ADR 0036](decisions/0036-seeded-run-transport-v1.md). Do not infer v2 semantics from a v1 request or rewrite those historical criteria. The SQLite store implements durable v2 operation arbitration. Memory and File stores do not implement that reservation and return typed unavailability for derive-once rather than pretending restart-stable support.

## Target support discovery and new submissions

Authenticated workflow-management clients with the `workflow:read` scope can use the query-free `GET /v2/workflow-targets` route to inspect the closed `ascension.workflow-targets/v2` catalog. Each target carries the exact actor-scoped V1 descriptor in `target` and its `descriptor_digest`. The response copies the V1 `catalog_revision` and reports the independent `support_revision: "seed-support.v1"`. This is a Harness-owned management contract, not a new `sts2-protocol` artifact. The V1 target route and response shape remain unchanged, as does the V2 workflow-run request.

The catalog keeps durable support separate from readiness and launch setup. `durable_candidate_binding_modes` reports structural storage support: explicit mode requires durable seed-binding storage; derive-once also requires durable operation reservations and a configured key authority. A configured authority with `Unknown` readiness can therefore remain structurally advertised. `ready_candidate_modes` is a discovery hint: it is empty for an unavailable target, and derive-once appears only for an available target whose authority reports `Ready`. A new derive-once submission must still obtain the current key and fails closed if retrieval is unavailable. `supported_launch_setups` is always empty in this schema; candidate-binding support does not establish launch-setup support.

Before lookup, derive-once also requires a store that supports operation-reservation lookup; a store without that capability fails closed because it cannot inspect or recover persisted derive-once operations. On stores that support the lookup, the additional structural durable-mode gate applies only to a new submission whose initial operation or binding lookup is `Missing`. After current owner target admission is revalidated, this gate runs before inference-profile admission, current-key retrieval, reservation or binding persistence, and execution. It does not move ahead of historical lookup: `CandidatePersisted` follows the existing exact submission path, while a `Prepared` derive-once operation recovers from its stored admission and pinned historical key version. Readiness is not a key-rotation mechanism. The file authority is an immutable startup snapshot; changing the keyring file does not rotate the running process, and recovery still requires previously pinned versions.

## Service key authority

The served workflow process optionally reads STS2_SEED_DERIVATION_KEYRING_PATH. This variable contains only an absolute path (at most 1024 bytes), never a key. If it is absent, derive-once is unavailable. The process loads an immutable keyring snapshot at startup; changing the file does not rotate the running process. Restart the service to select a new current version.

The file is ASCII with LF line endings and this closed format:

    schema=ascension.seed-keyring/v1
    authority_id=example-service
    current_version=key-2026-01
    key.key-2026-01=<64 lowercase hexadecimal characters>

The angle-bracket value is explanatory only; replace it with secret key material before use, and never copy a real key into documentation, requests, readbacks, logs, or shell history. Key IDs contain 1–64 ASCII letters, digits, period, underscore, or hyphen. The file is limited to 64 KiB and 64 versions; duplicate or unknown fields, comments, CRLF, malformed keys, and a missing current version are rejected. The current version must have a key entry.

The current adapter supports Linux only. It opens the file without following symlinks and requires a regular file owned by the service UID, a single link, owner read access, no group/world permissions, and no executable bit. The containing keyring directory must be service-owned and owner-private; traversed ancestors must meet the adapter's trusted-owner/mode rules (the root-owned sticky /tmp exception is exact). Windows and other platforms fail closed until equivalent checks are implemented.

Use a protected secret-management process to place the file; this feature does not provision keys. Keep every authority/version needed by persisted operations and bindings, including completed operations that can still be replayed. The current store has no key-retirement policy, so do not remove old versions. A missing or changed pinned version fails closed; rotation selects a new current version only for new operation identities.

## Current boundary

The current service can durably reserve and read back a derive-once candidate, including an awaiting_host_context state. The separate trusted host-context inspect/materialize integration and successful native start/readback remain outstanding. Source and synthetic tests do not establish host or game acceptance; no native run is claimed here.

Relevant implementation: [seed_key_file.rs](../crates/harness/src/management/seed_key_file.rs), [workflow_service_seed.rs](../crates/harness/src/bin/runtime_support/workflow_service_seed.rs), [service_seed_v2_submit.rs](../crates/harness/src/management/service_seed_v2_submit.rs), and [store_sqlite_seed_operation.rs](../crates/harness/src/management/store_sqlite_seed_operation.rs).
