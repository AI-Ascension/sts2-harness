# ADR 0036: Seeded-run transport and startup handoff

- **Status:** accepted for the harness seeded-run-v1 integration
- **Owners:** harness coordinator; gateway, MCP, and game-mod owners retain their boundaries
- **Scope:** one explicitly configured standard Ironclad seeded run per Runtime-v3 episode

## Decision

The harness treats seeded admission as a separate capability from the Runtime-v3 gameplay
profile. When `STS2_SEED_PLAN_JSON` is absent, the existing gameplay lifecycle is unchanged. When
the plan is present, `RuntimeConfig` requires the bounded environment fields described in the
seeded-run operations document and constructs one validated `SeedTransportConfig`.

The plan contains contiguous ordinal entries and preserves each `requested_seed` byte-for-byte.
The selected context is concrete and digest-bound: standard mode, Ironclad, ascension, ordered
modifiers and acts, selection policy, profile baseline, save policy, and game/mod compatibility
identities. The context digest excludes only its own digest field. Harness-only `plan_digest` and
`entry_ordinal` stay in the local reservation and receipt; they are never forwarded through the
seeded MCP/native request.

After gateway allocation and normal Runtime-v3 MCP initialization, the harness performs one
read-only `sts2.observe` to establish the host generation fence. A failed or not-yet-ready
observation fails preflight, closes owned MCP processes, and releases the lease before creating a
seed reservation or sending a mutation. The controller or launcher may wait for the host's native
campaign setup readiness before this handoff; that readiness wait does not authorize an extra seed
start.

The seed reservation is created atomically at `STS2_SEED_RESERVATION_PATH` before spawning the
seeded MCP profile. A new reservation permits exactly one `start_seeded_run` mutation. A matching
existing `start_pending`, `unknown`, or `settled` reservation enters reconcile-only recovery and
uses the original request generation. A malformed or conflicting reservation fails closed.

The seeded MCP profile is `seeded-run-v1-mcp`. Its only tools are `start_seeded_run`, mapped to one
native POST, and `reconcile_seeded_run`, mapped to a read-only GET. An uncertain start is recorded
as unknown and may create a fresh seeded MCP process only for reconciliation with the same
operation ID. It never retries start with a new operation identity. A settled result requires the
native canonical seed lineage, a fresh advanced observation, and a `run_started` effect witness;
an accepted response is admission only.

The `start_seeded_run` MCP arguments follow the catalog exactly: the requested seed is sent as
`seed`, while `selected_context` carries its validated `context_digest`. The top-level MCP
arguments contain no `requested_seed`, `context_digest`, `plan_digest`, or `entry_ordinal`; the
MCP mapper converts `seed` to the native protocol's `requested_seed` and derives the native
`context_digest` from `selected_context`. The harness still preserves `requested_seed` in its
reservation, receipt, and native response validation.

When `STS2_SEED_VERIFY_IDEMPOTENCY=true`, the harness sends one exact duplicate start request with
the same operation, lease, context, and generation identity after settlement. It compares the
canonical seed, observation, and effect witness, then performs a read-only reconciliation. This is
an optional verification exchange, not a second run admission.

The settled seeded receipt is emitted before gameplay observation and carries requested and
canonical seeds, context and operation identity, reservation metadata, status, generation, raw
bounded MCP exchanges, and host witnesses. The gameplay Runtime-v3 profile uses the same allocated
instance and lease but does not issue another seeded start. Shutdown closes seeded, expert, and
normal MCP processes before release confirmation.

## Restart and recovery limits

- One seeded mutation is permitted per operation identity.
- Reconciliation is bounded by `STS2_RUNTIME_SETTLEMENT_TIMEOUT_SECONDS`; timeout remains unknown
  and requires a later reconciliation with the same operation ID.
- A restarted controller may resume only a matching durable reservation and its original request
  generation. It cannot select another plan entry or advance the fence during recovery.
- A seeded MCP replacement is limited to read-only reconciliation after an uncertain exchange.
- The harness does not infer host settlement from process readiness, an MCP acknowledgement, or an
  HTTP success response; the host observation and effect witness remain authoritative.

## Consequences

This keeps seed admission, gameplay control, host authority, and durable recovery as separate
responsibilities. The harness owns bounded configuration, reservation, receipt validation, and
handoff ordering. The gateway owns lease and generation fences, MCP owns framing and mapping, and
the game-mod/host owns canonicalization and the run-start effect. The protocol artifact and
cross-target conformance files are checked separately from this coordinator policy.

The implementation is source and deterministic-test evidence. It does not by itself establish a
live host run, Windows/Linux compatibility, or a completed gameplay campaign.

## Amendment: consistent new-seed admission with retained legacy recovery

The runtime plan now uses the same `seed_binding::canonicalize_seed` validator as explicit authored
workflow requests. Every seed in a new plan must be non-empty, at most 64 UTF-8 bytes, free of control
characters, and free of leading or trailing Unicode whitespace. Interior whitespace remains valid.
Accepted bytes are preserved; this policy neither trims a seed nor owns the host's canonical-seed
readback. The duplicate runtime input bound is removed.

Earlier runtime plans accepted surrounding whitespace. Rejecting every such plan would prevent
same-operation reconciliation of a possibly delivered legacy start. A plan containing only that
historical difference is therefore classified as **recovery only**. Configuration admission requires
an existing `seeded-run-reservation-v1` record matching the complete operation, plan, selected entry,
seed, mode and context identity. The existing lifecycle, receipt-presence and safe-generation rules
remain enforced. The same record is checked again when the reservation is used.

A recovery-only configuration never enters `create_new`: a missing, replaced, malformed or
conflicting reservation is an error, including deletion after configuration admission. A valid
record returns `resumed = true` and its original generation, so the existing runtime branch performs
only reconciliation. No fresh seed operation is admitted from noncanonical input.

This is a Harness-local admission correction. It changes no shared protocol schema, artifact digest,
management request shape, persisted reservation shape, accepted seed bytes or host authority. See
[seed admission and recovery](../seed-admission.md) for migration and verification scope.
