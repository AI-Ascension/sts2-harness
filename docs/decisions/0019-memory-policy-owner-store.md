# ADR 0019: authenticated durable memory-policy owner

Status: accepted by the coordinating owner for the memory-only Harness #95 component slice,
2026-09-14. This does not close #95, provider-session migration or Studio #119.

## Decision

`context_memory::policy_owner::MemoryPolicyOwner` is an opt-in, transport-free entrypoint backed by
a separate encrypted SQLite journal. It owns immutable original and target policy encodings,
target-bound reviews/approvals, explicit active-policy adoption, receipts and active-policy
preparation. It does not invoke a provider, generate a summary, resume gameplay or implement a
Console transport.

`MemoryPolicyAuthority` composes an authenticator, trusted clock and synchronized owner state.
The selected descriptor and its independent pins, actual corpus, phase2 revision/epochs and
scoped grants come from trusted composition, never the command. Authentication precedes the state
lease; current grants are checked inside it. A workflow scope alone grants no policy permission.
The concrete authority's maintenance update port serializes all relevant mutations.
Every successfully published update, including a no-op, advances its authority-owned owner epoch
with a checked safe-integer increment, preserving a supplied larger trusted epoch. Failed updates,
epoch rollback and exhaustion publish nothing. Old reviews/active bindings remain stale even when
corpus reconstruction, legitimate disable/reset or descriptor/revision changes return other fence
values to an earlier state. Maintenance therefore requires explicit revalidation/approval/adoption
before further preparation; callers must use inspection for read-only health checks.

The approved atomicity boundary holds the owner-state lease through a bounded SQLite immediate
transaction. This is a deliberate local persistence exception to the general rule against blocking
I/O under a lock. No provider/network callback runs under that lease. Clock implementations and
trusted maintenance closures must be bounded and non-reentrant. Time is not frozen by the mutex:
grant expiry is checked again immediately before transaction commit.

An adapter must not substitute an independently mutable corpus/control snapshot and claim the same
guarantee. Distributed owner composition requires a separately accepted lease/fencing protocol.
No product dependency on Context Console is introduced.

## Policy and approval identities

The existing closed policy v1, migration proposal v1 and capability v3 contracts are unchanged.
Raw history SHA-256 covers exact supplied bytes; execution SHA-256 covers typed serde encoding.
The existing migration helper's adopted-policy hash retains that execution meaning.

Import stores schema-valid policies without activation. Migration proposes an independently
authored newer bounded policy for a genuinely over-limit saved source. Revalidation is a separate
internal review kind: it requires a previously adopted active identity and permits identical raw
bytes/version or a strictly newer independently authored target. It never fabricates a migration
violation, rewrites old bytes or silently downgrades a policy.

The original duplicate-rejected JSON must pass the complete embedded wire schema before typed
deserialization can supply optional defaults. Portable semantic and scope checks remain additional
requirements, including on authenticated recovery. Schema-valid but unsupported numeric encodings
fail explicitly without coercion; stored bytes and existing contract schemas remain unchanged.

Explicit approval binds the reviewed source/target hashes, current state fences and authenticated
subject/grant epoch. Adoption requires that same subject and grant with both approve/adopt
permissions. It revalidates the target against the actual corpus and selected descriptor, including
exact current phase2 revision and corpus generation. Activation, adoption history and receipt commit
together. An approved status inside policy JSON is not approval authority.

`prepare_active` loads the durable target itself; its request contains no policy. It validates the
active fences, current selector grant and continued approval-grant validity, then calls actual
memory retrieval/selection. It also admits query and final prepared-byte limits. It returns a local
selection manifest, not provider dispatch authorization. Existing runtime callers are not silently
switched to this component.

## Persistence and compatibility

The new store is `ascension.context-memory.policy-store.v1`, numeric version 1. It has no automatic
upgrade/downgrade or history eviction. Unknown schema versions fail before writes. Records are
logically append-only inside one authenticated encrypted bounded journal; its active pointer and
receipts change atomically. Every rewrite uses a fresh random nonce and scope/schema-bound AAD.
Synthetic-only construction is explicit; private retention still requires independent accepted
policy and key provisioning.

Open checks file size, actual SQLite page size/count and ciphertext length before loading the
encrypted blob. Typed decoding bounds record counts. Key/history validation precedes ownership
claim and is repeated inside the claim transaction. Replacement increments a persistent epoch;
stale handles cannot reclaim it. Restart preserves inspection but fences preparation until explicit
revalidation, approval and adoption. A wrong-key or corrupt-history open cannot take ownership.

Exact original v1 table definitions and the complete set of SQLite schema objects are checked before
claim and within journal transactions. Unexpected triggers, views, indexes and altered definitions
are incompatible, regardless of ciphertext validity. Metadata text is bounded in bytes before
retrieval. Writes require one affected row and exact candidate epoch/ciphertext read-back before
acknowledgment; this defense does not authorize otherwise incompatible schema behavior.

Same subject/idempotency key with the same request recovers the original receipt, including after a
lost reply. A changed request conflicts. Recovery does not reactivate a fenced policy.

See [memory-policy-migration.md](../memory-policy-migration.md) for bounds, operations and component
verification. Future Console/Studio migration capabilities, commands/routes and authenticated
principal mapping need a separate coordinated contract. Existing capability sidecars remain the
record publication seam; this ADR adds no effective-limit record endpoint.
