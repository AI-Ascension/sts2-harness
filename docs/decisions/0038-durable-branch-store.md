# ADR 0038: Durable Checkpoint Branch Metadata Store

## Status

Accepted for the harness component boundary. This decision authorizes durable branch metadata,
immutable ancestry, and retention references in a local SQLite store. It does not authorize native
checkpoint restoration, replay execution, gateway lease allocation, or game-process access.

## Context

The in-memory `BranchTree` foundation keeps sibling identities, parent edges, and writable scopes
distinct, but it cannot survive an owner restart. Issue #116 also needs a durable fork occurrence,
strategy descriptor, independent run associations, lifecycle evidence, event cursor, and artifact
reachability record. The harness owns those records; the game-mod owns native state and effects, the
gateway owns destinations and leases, and the artifact store owns blob deletion.

## Decision

`SqliteBranchStore` owns the versioned `ascension.durable-branch/v1` model. A branch record keeps
experiment/root identity, immutable parent and fork occurrence, exact state digest, selected
`exact_restore` or `prefix_replay` strategy, source handle/trajectory prefix, effective seed/setup,
boundary, assurance, run/episode/trajectory/context associations, policy/config revisions, operator
metadata, lifecycle status, timestamps, and artifact references. Gameplay state identity is never
used as a branch key; equal state digests remain separate occurrences.

Creation is metadata-only and starts at `pending`. One SQLite transaction inserts the experiment
root or an existing-parent child, its append-only occurrence projection, immutable branch row,
operation receipt, artifact edges, and `created` event. A repeated operation ID and identical
payload returns the existing branch; a changed payload fails with `IdempotencyConflict`. Parent,
run, branch, scope, occurrence, label, branch-count, and artifact-count checks happen before
commit. SQLite foreign keys enforce experiment, parent-branch, occurrence-parent, and artifact
ownership relationships.

The lifecycle is `pending`, `restoring`, `replaying`, `ready`, `running`, `held`, `completed`,
`failed`, `unknown`, and `archived`. Readiness requires strategy-specific evidence:
`exact_restore` requires `exact_restore_receipt`, while `prefix_replay` requires
`prefix_replay_boundary`. These labels are not interchangeable. Startup reconciliation resolves every
half-created row deterministically: a `pending` fork intent that never started a strategy is
archived, while a `restoring`, `replaying`, or `unknown` attempt fails closed as `failed`; it never
retries an uncertain effect or allocates a destination.

Selecting a `ready` branch for a fresh continuation uses a metadata-CAS transition back to its
strategy preparation state (`restoring` or `replaying`) before any destination effect. That
transition clears the previous strategy assurance; the runtime must publish fresh destination
evidence and return through `ready` before it admits live decisions. A crash in this selected
attempt while it is `restoring` or `replaying` is resolved by the same fail-closed startup policy
and never repeats the effect. A branch left `running` is refused by explicit selection and remains
non-executable: automatic recovery requires persisted ownership evidence tying the continuation
operation to the destination's current lease/session fence. That evidence is not currently exposed
by the gateway/runtime owner, so startup does not mark a potentially live sibling failed or replay
it. Operator reconciliation is required until that owner contract exists.

Reads provide stable branch-ID pagination, root-first ancestry, and append-only event cursors. Rename,
assurance, lifecycle, and artifact mutations use operation IDs plus a metadata CAS revision.
Archiving is reversible through `archived -> pending` and does not remove dependencies. Explicit
pruning requires archived (or policy-eligible completed) branch IDs, writes branch and artifact
tombstones, and reports retained versus collectable artifact references. A collectable reference is
only one with no remaining non-pruned branch root; actual blob deletion remains an artifact-owner
operation and must be planned separately.

The branch graph records only opaque artifact identities, so the store never assumes that a retained
reference is still readable. Resolving availability delegates to the artifact owner through
`BranchArtifactResolver`; the harness exact store answers with `available`, `missing`, or
`unverifiable`. Identity outside the verified manifest/blob namespaces, bytes that no longer match
their recorded identity, and store failures are all reported as `unverifiable` rather than
`available`, because each of them blocks a continuation and none of them authorizes recreating the
artifact. Resolution reads and verifies bytes and never writes, so an absent artifact stays absent
instead of being re-derived or replaced by a fresh start. Availability is necessary but not
sufficient for continuation readiness: strategy evidence, scope, destination ownership, and leases
remain separate gates. A tombstoned branch reports `ArtifactUnavailable` rather than a vacuously
available empty remainder, because an explicit prune has already collected its retained edges.

## Persistence and migration

Revision 1 creates the branch tables transactionally and records a namespaced `branch_schema_meta`
row. It deliberately leaves SQLite's global `user_version` untouched so the branch tables can share
a file with other harness stores. Opening a newer revision fails closed with `UnsupportedSchema`;
opening an older or empty database applies only forward, idempotent creation. There is no automatic
downgrade: operators must retain a backup and restore it through the owning deployment procedure.
The branch database may share a local SQLite file with other harness stores, but its table names and
contract revision are isolated.

## Compatibility and evidence

This is an additive, owner-local component contract. Future serialized consumers must pin the
contract version and fixture digest before claiming compatibility. The fixture in
`conformance/durable-branch-v1/valid.json` records the bounded vocabulary and synthetic provenance.
`durable_branch_store.rs` tests cover restart recovery, equal-state siblings, operation idempotency,
unknown parents, strategy assurance, metadata CAS, shared-artifact retention, tombstones, event
cursors, and future-schema rejection. These are deterministic source/component checks only.
Native receipt production, replay boundaries, host/profile isolation, leases, live effects, and
release compatibility remain unverified.
