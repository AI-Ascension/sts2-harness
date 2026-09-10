# ADR 0013: Context-control durable store

Status: accepted for the Phase 2 harness companion seam. Owner: harness maintainers.

`ContextControlStore` is an opt-in, owner-local SQLite store for the context-control authority
journal. It is a companion seam for the console fixture; it does not grant the harness authority
over a provider, MCP server, gateway, host, game, watchdog, or service installation. The target
console remains bounded and in-memory until it adopts this seam explicitly.

The store uses one local SQLite connection per owner and `BEGIN IMMEDIATE` for journal, outbox, and
Phase 1 snapshot writes. SQLite is configured with WAL, `synchronous = FULL`, and foreign-key
checks. The journal envelope and its digest, management-active marker, pause/control/plan epochs,
active revision, and outbox rows commit atomically. A failpoint immediately before journal write or
transaction commit verifies that an error leaves the previous durable state intact. The store does
not call external code while a transaction is open. SQLite and filesystem guarantees remain
conditional on the host OS, storage device, and process ownership; this is deterministic local
evidence, not a native crash or deployment guarantee.

Journal bytes are serialized by `ControlAuthority`, encrypted with XChaCha20-Poly1305 using a
caller-owned 32-byte key and fixed associated data, and stored as an opaque envelope. The key is
never persisted by the store. Envelope length and SHA-256 digest are checked before decryption;
authentication failure, corruption, malformed journals, scope mismatch, and oversized inputs fail
closed. Outbox payloads contain bounded control events only; private note or capture content is not
written by this seam. Phase 1 snapshots are copied into an additive immutable table with a digest,
so the historical bytes are retained and a conflicting identity cannot overwrite them.

The schema is independently marked as `ascension.context-control.sqlite.v1`. Opening a database
with the same marker repairs a partially applied additive table creation in one transaction;
unknown or newer markers are rejected. A copied database can be backed up after a full WAL
checkpoint. `legacy_open` refuses a management-active store and permits a disabled marker, so a
legacy binary cannot silently ignore active management state. Disabling the marker does not delete
the journal, revision, pause, or Phase 1 snapshots.

Recovery decrypts and replays the authoritative journal and increments the controller incarnation
inside the recovered authority. Restart therefore does not auto-resume a paused or committed run;
the caller must apply its normal fresh-preview and ownership policy. This seam does not yet provide
OS-level exclusive leases, prepared-provider claims, ambiguous-write reconciliation, or a native
target-console integration. Those remain unverified and are intentionally reported separately.
