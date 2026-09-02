# Runtime-v2 deterministic fake evidence

Date: 2026-09-02
Evidence level: `test-confirmed` deterministic in-memory fake only

## Scope

This record covers one `sts2-harness` fake instance using the frozen Runtime-v2 artifact handed
off by `sts2-protocol` commit `8d4b2f574cf860a71f2a5e4ce3308ac069cb1527`. The copied schema and
source bytes have digest
`f7963b19c8ed5bbdc02c08e83c7a2e16c4771ed5eb798b29a8208d7a917a86c2`.

The owner-local copy is under `protocol-artifact/runtime-v2/`. Its `manifest.json`, `schema.json`,
19 sanitized golden messages, source schema, conformance case, and `SHA256SUMS` are checked in.
The harness includes these local bytes and verifies their manifest, schema identity, source/package
byte equality, and every checksum before running the fake. No sibling checkout or filesystem/path
dependency is used.

## Deterministic lifecycle

The executable `sts2-harness-runtime-v2-fake` binds `instance-1`, `session-1`, `lease-1`, and epoch
`1`. It begins at generation `4` with a player-turn observation at turn index `2`.

1. It allocates `op-1` before constructing or submitting the `end_turn` request and records
   `requested`.
2. The fake admits the request and records `accepted`; this is admission-only at generation `4`.
3. A deterministic post-write disconnect applies exactly one in-memory mutation, returns no result,
   and records `unknown` with `retry_attempts: 0`.
4. The harness calls the fixed `reconcile_action` path with the same `op-1`, records
   `reconciled`, and receives `settled` with a fresh generation `5` observation and
   `turn_end_settled` witness.
5. A fresh state read confirms generation `5` and turn index `3`.
6. Replaying the same action request returns the stored settled result and leaves the mutation count
   at `1`.
7. A request using epoch `0` is rejected as `sts2.gateway/stale_lease_epoch` before mutation.

Trajectory records retain the Runtime-v2 envelope, provenance, operation identity, action, status,
observation, witness, identity fence, and explicit no-blind-retry evidence. The trace artifact binds
its content digest to the trajectory and its schema digest to the verified copied bytes.

## Unverified lanes

This wave does not launch or contact STS2, a game host, a game mod, a provider, a model, a profile,
a save, or a network service. The fake proves local contract and lifecycle behavior only. Live host
settlement, gameplay mutation, host/API compatibility, MCP/gateway runtime integration, provider
execution, model behavior, persistence, and production artifact publication remain `unverified`.

## Reproduction

Run from the repository root:

```text
(cd protocol-artifact/runtime-v2 && sha256sum -c SHA256SUMS)
cargo run --locked --offline --package sts2-harness --bin sts2-harness-runtime-v2-fake
cargo test --locked --offline --package sts2-harness --test runtime_v2
```

The first command reports 23 `OK` lines. The binary emits one deterministic JSON document and exits
`0`; the integration test repeats it and checks byte equality, lifecycle outcomes, fencing, replay,
and artifact lineage.
