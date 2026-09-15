# ADR 0021: Immutable benchmark manifest foundation

Status: implemented harness-owned library candidate; independent review pending.
This is partial delivery for #121.

## Boundary and requirements

The harness owns private benchmark configuration identity and experiment lineage. This additive,
opt-in library has no runtime, filesystem, allocation, seed-generation or provider operation.
It consumes the existing seeded-run-v1 artifact; game-mod remains seed/state authority, gateway
remains profile/lease/process authority, and MCP remains transport authority.

Required observable behavior:

1. Parse a closed, versioned, bounded manifest, reject duplicate keys at every depth, and produce
   deterministic immutable bytes and domain-separated SHA-256 identities.
2. Keep controlled gameplay inputs, experiment inputs and trial occurrences distinct. Exact
   comparison rejects every changed input without compatibility exceptions or mutation.
3. Require requested seed and expected effective seed separately. Preserve both byte-for-byte.
   Require a declared normalization/derivation contract; do not implement or infer normalization.
4. Bind a planned occurrence to an existing settled seed receipt only when request fence, complete
   available wire identity, requested/effective seeds and selected context match. This is
   `seed_receipt_bound`, never authenticated native or complete gameplay/RNG verification.
5. Expose only a keyed opaque reference and fixed evidence labels publicly. Private seeds, profile
   references, exact digests, inference inputs and raw receipt bytes never enter that projection
   or diagnostic output.

## Contract and identity

`ascension.benchmark-manifest.v1` uses required fields with no implicit defaults. Required gameplay
inputs cannot be unknown/null. Provider revision and sampling-seed availability are explicit
closed alternatives; absent metadata is rejected, and unavailable guarantees do not imply
deterministic sampling. Provider inference parameters are a bounded string map recording the
provider's exact declared lexical values; this library neither interprets nor executes them.
Budgets are explicit positive bounded integers, never missing-as-unlimited.

The gameplay input includes the accepted seeded-run selected context (mode, character, ascension,
ordered acts, sorted unique modifiers, selection/save policy, profile baseline and game/mod
identities), expected effective seed, declared seed contract, explicit game version, assembly hashes, independent
component revisions/packages, protocol/coverage versions, OS/architecture/runtime, gameplay
settings and unlock/progress configuration digests. A pristine profile artifact reference binds
the separately stored private baseline rather than embedding a save or path. These are declarations,
not completeness or compatibility attestations.
Compatibility digests and component package digests are independent axes: the accepted artifact
does not establish that they hash the same bytes. Settings/unlock digests bind separately retained
private configuration; this foundation does not reconstruct or apply that configuration.

Configuration identity hashes only gameplay inputs, including the requested seed as a controlled
startup input. Experiment identity additionally binds provider/model/revision, prompt, workflow,
context and tool-policy digests, inference parameters/seed guarantees, budgets and evaluator.
Artifact identity binds the entire versioned manifest. Trial process/run/episode/session/lease/
operation identities and timestamps are excluded from all three. Ordered arrays remain ordered;
object member order and JSON whitespace do not change identity. No equality relaxation exists.

Canonical v1 encoding is compact serde JSON of the closed typed records in declared field order,
with lexicographically sorted string-map keys, preserved strings, integer numbers and no floats.
The domain/version is included in each digest preimage. This is a harness-owned encoding, not JCS
or a change to seeded-run-v1's existing context digest encoding.

## Verification, persistence and compatibility

The immutable manifest exports private canonical bytes for an authorized artifact adapter.
A planned trial keeps the manifest and expected occurrence together. Binding returns a new
receipt-bound record; failure cannot alter the manifest or planned record. The retained receipt
can be checked again with the same plan. A new run needs a new occurrence; no API redraws seeds.
This in-memory contract does not claim atomic durable publication or crash/lost-reply recovery.
Those remain with #103 and the existing seed reservation/runtime integration.

Receipt structural checking reuses the accepted schema and legacy recorded-run seed consistency
check, then adds exact planned identity/context matching. No offline parser authenticates a host.
The legacy receipt wrapper requires `operation_id`, `requested_seed`, `plan_digest`,
`entry_ordinal`, `settled`, `start` and `reconcile`; only optional `start_error` and
`duplicate_start` are accepted additionally. Unknown wrapper semantics fail closed.
Raw start/duplicate/reconcile exchanges are bounded opaque private archival data, not verified
MCP history or evidence. The current contract allows at most 64 retained reconcile exchanges.
The wire carries no harness run/episode/process or MCP-session identity. The owner must preserve
unique operation/fence association across runs; this library cannot distinguish illicit reuse
of the entire same tuple under a different harness-only run identity.
The accepted receipt does not attest assembly inventory, OS/runtime, unlock progress or complete
hidden RNG. The new state therefore verifies only the available receipt binding.

Classification: additive-compatible owner library/API and new private format; no old record,
frozen protocol artifact, CLI, database migration or provider default changes. Legacy records
retain their original interpretation. Unsupported versions, unknown fields and malformed inputs
fail closed. Rollback removes this opt-in library use; it never reads v1 as a legacy format.

Tests use hand-authored MIT synthetic configuration and the existing MIT seeded-run artifact.
They must cover canonical identity, each controlled field, experiment/occurrence separation,
strict malformed/oversize/duplicate boundaries, mismatches, receipt substitution and projection
privacy. Source/offline tests are not game/provider evidence.

Full #121 acceptance still requires #103 seed persistence integration, #79 native admission,
owner-agreed complete readback/RNG and compatibility contracts, profile provisioning and a
separately authorized cold-launch evidence lane. No full issue closure is claimed.
