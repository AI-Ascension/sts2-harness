# ADR 0020: Exo owner journal and single-use send boundary

Status: accepted for the opt-in source persistence slice; production runtime admission remains unsupported.

## Context

The provider-session broker already owns session identities and restart fencing. Its v1 encrypted
metadata store does not supply a lifetime cross-process owner lease. The execution store already
owns provider reservations and exact completed result bytes. A possible provider write must not
become a second model call after an uncertain filesystem operation or restart.

This increment addresses part of issue #142. It supplies no native Exo executor, managed process
protocol, scheduler authority implementation, accounting adapter, cancellation implementation,
or enabled runtime profile. Fixtures use synthetic identities and usage values.

## Decision

The broker owns a distinct encrypted v2 journal directory containing `journal.enc` and a permanent
`owner.lock`. It never overwrites a v1 destination. A required authenticated authority port
authorizes create, restart, legacy cutover, effect admission, and result consumption. No default
allow adapter exists. The caller must bind every manifest field to its current scheduler owner,
authorization, lease, game state, package, profile, and frozen configuration.

An exclusive OS file lock on the separate lock inode lasts for the owner object's lifetime.
Descriptors use close-on-exec and are not cloned or exposed. Paths are absolute and traversed
without following symlinks; final directories and files must be private and owned by the
effective user. Reopening verifies both directory and lock inode. Unsupported locking fails
closed. The Unix source adapter is a local filesystem boundary, not a claim about network
filesystem locks or crash durability on every platform.

The v2 envelope uses XChaCha20-Poly1305 with a distinct magic and authenticated scope/store
identity. Its payload includes the existing broker snapshot, claim epoch, monotonic revision,
immutable invocation metadata, and result references. The input and result payload are absent.
Each commit verifies the live lock and the authenticated persisted epoch/revision, writes a
private exclusive temporary file, fsyncs it, renames it, and fsyncs the containing directory.
Every uncertain commit poisons that owner permanently. New directory creation also syncs its
parent. No lock file is unlinked as recovery.

The journal retains at most 128 entries, additionally bounded by broker policy, and 4 MiB of
serialized plaintext. Unknown entries and terminal tombstones count toward the bound. IDs and
cursors are at most 128 ASCII bytes; input is at most 131072 bytes. The existing execution-store
8192-byte result bound remains authoritative. There is no eviction, compression escape hatch,
new budget ledger, or second result database.

## Send and completion

The frozen request envelope is checked against the manifest, legal-action catalog digest, and
the broker's exact prepared-input operation digest. Existing execution-store readiness gates
run before fresh journal intent. The owner persists `Prepared`, records the existing decision
and reservation, persists `Admitted`, and persists `Sent/possible_write` before constructing a
non-cloneable, non-serializable send permit. An exact duplicate can reuse a completed result;
all other existing entries remain held.

The authority guard spans admission through `EffectPort::try_start`. That method must promptly
hand off and return a handle, without waiting for inference. Polling holds no authority guard,
so cancellation or revocation can linearize after handoff. Every handoff or polling error is
ambiguous. Before consuming a terminal result, the owner validates its correlated envelope,
legal action, adapter-reported native identity shape, and positive usage within the reservation.
Authenticating native identity and qualifying usage remain the admitted adapter's responsibility.
Current
authority is checked again. Result bytes commit in the execution store before journal terminal
metadata; a failure in the latter cannot trigger another send.

The live owner and handle bind the existing execution store's in-memory incarnation.
Every store-taking path rejects a different incarnation before polling, changing a handle,
publishing uncertainty, or returning a result. Completion and uncertainty additionally require
the exact frozen reservation and decision metadata. Reopening the same database creates a
different live incarnation and cannot finish an old handle. A restarted owner's first exact,
currently authorized completed-store reconciliation establishes its store binding; it does
not restore a serialized incarnation or send permit.

This source slice consumes single-action decisions only. It does not implement plan execution,
provider interruption, native process containment, or cleanup. Those capabilities require
their own admitted adapters and tests before a complete runtime profile can be enabled.

## Restart and cross-store disagreement

Restart requires a fresh authenticated claim and checked broker restoration with the approved
scope, policy, and capabilities. It increments and persists the owner epoch before returning.
The existing broker turns active bindings into recovering bindings and uncertain operations
into unknown operations. A send permit is never restored. Prepared, missing-reservation,
reserved, unknown, and conflicting cross-store records remain held without automatic repair
or allocation.

Authenticated journal entries must resolve to their exact broker binding and turn operation,
including the prepared-request digest, immutable operation epochs and compatible phase.
Validation precedes the restart authority claim and any claim/revision publication. Historical
completed operations retain their original epochs as bindings advance. Prepared/Admitted
records after restart correspond to held Unknown operations; completed-store repair may retain
an unfinished held broker operation. The reserved `FailedBeforeSend` enum has no writer in this
source slice and is rejected at the journal boundary. No malformed authenticated record is
repaired by incrementing the claim or rewriting the retained ciphertext.

An explicit `reconcile_stored` call may verify an existing completed transaction's exact
lineage, reservation, result digest, correlated response, and current consume authority, then
repair only journal metadata. The restored broker remains held. This is not full broker or
native reconciliation and cannot resume a binding or admit a fresh send. A reservation already
marked Unknown cannot be forced to Completed through this interface. Missing, zero, or
over-reservation usage stays held; no usage value, refund, or native ID is invented.

## Legacy migration

Migration requires an authenticated claim that proves the v1 owner is stopped and its old
execution route is permanently revoked. A caller boolean is not a production attestation.
The importer reads one bounded no-follow descriptor, verifies the expected encrypted source
digest, uses the existing v1 decrypt/checked-restore implementation, and creates a new v2
destination with the import digest and cutover reference. Legacy bytes remain unchanged.
Repeated migration cannot replace the destination or reset its history. A resumed old v1 writer
cannot modify the separate v2 journal, but preventing that writer from executing is the
authority owner's responsibility.

## Relationship to ADR 0017 and remaining admission

A future managed Exo adapter uses a fresh native conversation for each logical decision and
never depends on native conversation history as game authority. This refines ADR 0017 for this
bounded executor path. No production adapter is wired here. Native cancellation, terminal
evidence, qualified accounting, containment, scheduler composition, and full episode admission
remain separate work. Source tests and local OS process contention tests prove only their
declared persistence boundaries; injected I/O failures do not prove physical power-cut behavior.
