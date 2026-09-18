# ADR 0051: Logical-Invocation Context Lifetime Consumption at Dispatch Admission

## Status

Accepted for the Harness context-control boundary. This record does not
authorize a provider, native-host, game, deployment, or paid-call lane, and it
does not give the harness authority to erase or relabel provider-session
history. It records how a bounded context lifetime is consumed, persisted and
audited.

## Context

[ADR 0046](0046-invocation-context-membership.md) made context membership
invocation-specific, and `ContextItem` already carried `expires_at` with
expiry/revocation filtering in context-memory. Wall-clock expiry alone does not
implement turn-scoped inclusion: it cannot express "these items belong to this
invocation and the next N", and it cannot stop a preview, a browser reload, a
receipt lookup or a transport retry from appearing to extend applicability.
Nothing durable recorded *which* logical invocations had actually consumed a
window, so a restart could not distinguish a consumed window from a fresh one.

## Decision

A continuity owner issues a **`ContextLifetimeScope`**
(`ascension.context-control.lifetime.v1`) for one agent/episode/run, optionally
pinned to one branch, over a bounded, ordered and unique set of item ids, with
an applicability of `CurrentInvocation` or `NextN { bound }` (bound ≤ 64, items
≤ 64) and a wall-clock `ceiling` that is an *additional* bound rather than the
mechanism.

Applicability is consumed at **durable dispatch admission**, on a
`LogicalInvocationIdentity` whose `invocation_id` is the logical identity and
whose `attempt` distinguishes transport retries.

1. **Consumption has exactly one site.** `ContextLifetimeLedger::admit` consumes;
   `preview` is a pure read. A preview, reload, receipt lookup or retry therefore
   cannot consume, extend or resurrect applicability.
2. **A retry is not a new invocation.** Re-admitting the same `invocation_id`
   (any `attempt`) returns the manifest that already exists instead of consuming a
   second slot. Expiry is evaluated at admission, so a replay after the ceiling is
   refused rather than honoured.
3. **The two sides of a crash differ.** A stop *before* the durable write consumes
   nothing. A stop *after* it leaves the slot consumed and the invocation **held**
   as a possible dispatch until reconciliation. Only `Released` returns capacity;
   `Dispatched` and `Held` keep it, because a dispatch that may have happened must
   not be silently handed back.
   Reconciliation is a **one-way** transition out of `Held`: re-settling to the
   settlement already carried is an idempotent retry, but changing it is refused
   (`context_lifetime_already_settled`). Without that guard a `Dispatched`
   invocation could later be released — handing back a slot whose dispatch did
   happen — and a `Released` one could be re-marked dispatched, leaving the live
   count disagreeing with the reloaded count.
   `ordinal` is the 1-based position in the scope's **admission sequence**, which a
   release does not rewind, so an invocation that refills a freed slot carries an
   ordinal above the declared bound while still being inside the window.
   `remaining_after` is therefore derived from capacity actually consumed rather
   than from the admission index.
4. **Counters and identities are persisted.** `DurableLifetimeState`
   (`ascension.context-control.lifetime-state.v1`) is written through the existing
   `ContextControlStore` in its own encrypted, digest-fenced, single-writer
   transaction, and reloads through `restore()`. The store's schema gains one
   additive table (`context_control_lifetime`); no existing table, column, digest
   or route changes. A reload cannot resurrect a consumed window.
5. **History is never rewritten.** Manifests are append-only and their digest
   binds the immutable admission facts, so reconciliation — the one field it may
   update — does not invalidate the record it settles. Expiry never deletes or
   rewrites a manifest, so an earlier effective-context decision stays
   inspectable under retention policy rather than being relabelled as erased.
6. **Scope is not inheritable.** A scope covers exactly its run, episode and agent;
   a branch-pinned scope covers only that branch, while a scope issued without a
   branch identity covers the same agent in any branch. Sibling agents, branches,
   episodes and runs are refused, and a refused sibling consumes nothing. A reused
   scope id must describe the same scope exactly, so a caller cannot rebind an
   existing scope to itself and inherit its remaining window.
7. **Approvals fence an exact revision.** `LifetimeApproval::verify` re-checks the
   ceiling at verification time and compares the exact manifest digest list, so a
   preview obtained before the ceiling cannot be replayed after it, and a later
   admission invalidates the approval instead of being silently absorbed.
8. **Counts are bounded and provenance is re-proved on reload.** One run holds at
   most `MAX_LIFETIME_SCOPES` scopes and `MAX_LIFETIME_MANIFESTS` manifests, so a
   window cannot grow until it is permanently unpersistable. Restoring re-proves
   every fact rather than trusting the image: each record must still bind its own
   canonical bytes, name the scope revision it is filed under, and agree with that
   scope about the window, items and owner.

## Honest limits

- `LifetimeManifest::verify` is an **integrity** check against corruption, not an
  authentication boundary: every input is public, so a caller that rewrites a field
  and re-derives the digest can always produce a self-consistent record.
  Authenticity comes from the surrounding envelope — a persisted manifest sits
  inside the control store's AEAD envelope under the store key, and a reload
  cross-checks the record against its scope's digest.
- The settlement field is deliberately excluded from the admission digest so a
  settled record still verifies, which means those bytes do not protect it. A
  caller that mutates `settlement` directly holds an unauthenticated record and must
  not be trusted to have minted capacity; `reconcile` is the only supported path and
  enforces the one-way transition.
- A scope issued without a branch identity deliberately authorizes the owning agent
  in any branch, which narrows branch isolation for that case. The live seam does not
  always reach a branch identity; an owner that needs branch isolation must pin one.

Strict omission from later effective provider context remains capability-gated by
the continuity owner; this record does not claim it, and local expiry never
deletes or rewrites historical manifests.

## Consequences

- Applicability becomes a durable, inspectable fact rather than a wall-clock
  inference, so a restart replays the same counters and the same identities.
- Consumers can prove *why* an invocation was inside a window from its manifest
  `ordinal` and `scope_digest` instead of trusting a counter.
- A held invocation can delay capacity return until reconciliation, which is the
  intended fail-closed trade: unknown dispatch is never assumed to have not
  happened.
- The lifetime state shares the control store's key, AAD, ownership fence and
  schema version, so it inherits that trust boundary rather than adding a second,
  weaker one. A future incompatible change must version this state rather than
  mutate it in place.

## Verification

Synthetic fixtures only; no provider, host, game or wall clock is contacted, and
all instants are explicit logical clock values with injected boundaries.
`crates/harness/tests/context_lifetime_{admission,nonconsumption,crash,isolation,durable}.rs`
cover the four acceptance criteria, including real SQLite reopen for the durable
half, failpoints on both sides of the durable write, tamper-evidence of the
persisted envelope, and refusal of a foreign identity.
