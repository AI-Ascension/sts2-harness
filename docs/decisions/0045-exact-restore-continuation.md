# ADR 0045: Exact-Restore Startup and Gameplay Continuation

## Status

Accepted for the Harness exact-restore consumer. The Harness owns durable branch
metadata and receipt admission. Gateway owns allocation and host fencing; MCP
owns the exact-restore transport and gameplay observations; the Mod owns native
state application. This record does not authorize a native game hook.

## Startup policy

A fresh exact-restore start may claim a `ready` branch only after the selected
manifest, canonical payload, restore artifacts, compatibility digest, and
coverage digest have been verified. The Harness records the claim intent before
calling the peer operation. The operation sequence is `begin`, bounded chunk
upload, `finish_blob`, `commit`, and receipt `lookup`; every request carries
the stable operation identity, expected owner fence, request digest, and
correlation identity.

The Harness accepts a verified receipt only when its operation identity,
manifest and blob digests, branch identity, original branch metadata revision,
closure state, and current owner fence all match the durable claim. Receipt
publication, the exact-restore assurance, and the owner `BoundaryVerified`
claim are persisted before the branch becomes `running` or gameplay is
started. A receipt is a destination fact; source checkpoint assurance cannot
stand in for it.

Native capability refusal is a pre-effect failure. The branch is marked
`failed`, no chunk or gameplay effect is accepted, and the next startup does
not retry that operation automatically. A transport or commit response that
could have crossed the host-effect boundary is `unknown`. Startup selection
refuses `unknown` (and an in-flight `restoring` claim) and requires an
explicit owner reconciliation procedure; it does not perform an automatic
lookup on startup. During an initial restore attempt, an ambiguous commit
response may use a same-operation, read-only `lookup`; if uncertainty remains,
later reconciliation requires owner action and is not an automatic startup
retry. A denied or stale-owner lookup cannot turn the prior uncertainty into
`failed`, and the Harness never blindly commits or creates a replacement
operation.

## Restart and continuation

Startup does not rerun exact restore for a branch already marked `running`.
Resume admission requires exactly one retained destination receipt, a
content-verified `ContextSnapshot`, `ExactRestoreReceipt` assurance, and a
current owner claim in `BoundaryVerified` or `Resuming`. The receipt's original
claim revision is found in the append-only event history with bounded
pagination; history after the first page cannot hide or replace that identity.
Any missing, tampered, foreign, expired, or revision-mismatched receipt is
refused before provider or gameplay startup.

After a verified receipt, the continuation route runs the owner-fenced gameplay
runner. It may observe state, choose a host-issued legal action, dispatch it,
and wait for the settled successor. The gameplay route never commits the
restore again. A gameplay `Unknown` result remains uncertain and is not marked
completed; a successful terminal report may advance the branch to `completed`.
Provider cleanup or durable completion failure is also reported as uncertain
when the effect boundary cannot be established.

## Evidence and CI

The deterministic unit matrix covers neutral MCP schema/correlation rejection,
native refusal, unknown-effect retention, receipt revision tampering, and
paginated claim history. The actual entrypoint test is ignored by default
because it requires pinned Gateway, MCP, and test-only Mod executables. The
checked-in `.github/workflows/exact-restore-conformance.yml` resolves and
verifies immutable Gateway, MCP, Mod, and Exo revisions, builds those peers
and Harness, then runs the Rust
`tools/exact-restore-conformance` launcher for positive, refused, and unknown
outcomes. It uploads retained sanitized child logs, synthetic effect counters,
the positive runtime action/settlement ledger, source revisions, and binary
hashes on every run. The positive outcome remains blocked until the pinned
Gateway typed translation is available; that is recorded as a conformance
failure rather than treated as a pass. An environment that omits a required
peer binary is a configuration failure, not a skipped exact-restore result.
