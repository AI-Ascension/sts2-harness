# ADR 0012: UUIDv4 Operation Identity at Action Creation

## Status and ownership

Implemented safety correction. The harness owns creation and retention of its operation identity;
MCP and gateway retain transport, lease, fencing, and authoritative outcome ownership. This
decision does not change the frozen Runtime-v3 gameplay artifact or recovery schema.

## Decision

Create a fresh UUIDv4 operation identity when each production episode or combat-demo action is
prepared. The identity is placed into the `ActionIdentity` before the first dispatch and is not
derived from generation, step number, action kind, or a model execution identity. A new process or
new action uses independent random identity generation even when its generation and step counters
reset. Production uses the pinned `uuid` crate's UUIDv4 generator; it does not synthesize UUID-shaped
values from timestamps, process IDs, counters, or general-purpose hashes. Uniqueness is probabilistic,
as with UUIDv4 generally; ledger conflict checks remain required.

The episode runner retains the exact identity through an uncertain dispatch, transport reconnect,
receipt reconciliation, and settlement verification. Recovery receives that same operation
identity; it never receives a replacement strategic action. The combat demo follows the same rule
for its direct runtime port path.

The state identity remains the exact state ID supplied by the authoritative host observation. The
harness validates that the legal-action catalog is bound to that observation before creating the
operation, but it never manufactures, rewrites, or substitutes a state ID to satisfy a recovery
validator. If the host state ID is not acceptable to a downstream recovery contract, that boundary
must fail closed and the owning host/protocol contract must be reviewed separately.

## Compatibility and evidence

This is a `safety-correction` to unreleased production paths. The frozen Runtime-v3 schema permits
bounded identity strings and its checked-in goldens remain byte-identical. The historical recovery
consumer separately requires UUIDv4 operation IDs, so generated IDs are checked through the actual
recovery-frame validator. Focused runner tests exercise production ID creation through dispatch and
reconcile, prove identity retention after uncertainty, reject conflicting action reuse, and verify
that fresh runner instances in one process receive distinct UUIDv4 identities. These tests do not
establish cross-process restart behavior. These are source/component tests; live
gateway, host, and cross-boot release-set compatibility remain unverified.
