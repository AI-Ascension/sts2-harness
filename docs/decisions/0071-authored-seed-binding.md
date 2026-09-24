# ADR 0071: Resolve and durably bind an authored workflow's seed

Status: accepted for the harness-owned, source-only slice of issue
[#103](https://github.com/AI-Ascension/sts2-harness/issues/103) — one bounded canonical seed per
logical run, drawn once per persisted record (no redraw once a record exists) and persisted before
any setup mutation, bound to the run's instance, baseline, lease and setup. It launches no game: native seed acceptance stays gated by
[sts2-game-mod#79](https://github.com/AI-Ascension/sts2-game-mod/issues/79). It is ratified when the
change carrying it merges.

## Context

Issue #103 requires an authored workflow to supply an explicit seed or a generate-once seed, resolve
randomness once per logical admitted run, and bind the effective seed to the run's target,
disposable-profile baseline, supported setup context and operation identity, so that duplicate
requests, lost responses and restarts never redraw or silently change configuration. The existing
seeded-run transport ([ADR 0036](0036-seeded-run-transport-v1.md)) already carries an explicit seed
and reconciles the same operation, but nothing in the harness owned the resolve-once, persist-before-
mutation and reject-before-mutation contract that produces that explicit seed.

Three failure modes had to be excluded by construction:

1. drawing randomness twice for one logical run after a duplicate request, lost reply or restart;
2. mutating setup before the effective seed was durably persisted, or persisting it and still
   redrawing on the next attempt;
3. accepting a seed bound to the wrong instance, a stale baseline or lease, an unsupported setup, or
   a conflicting prior seed.

## Decision

A new private module `crates/harness/src/seed_binding/` owns that contract, split so each file stays
inside the production size budget, and is additive to the existing seeded-run machinery:

- **Canonical seed (`canonical.rs`).** `canonicalize_seed` fixes the canonical UTF-8 form: non-empty,
  at most 64 bytes, free of control characters and with no surrounding whitespace. Multibyte
  characters count as their encoded byte length, so the bound is the same byte bound the seeded-run
  consumer already enforces; explicit and generated seeds normalize identically.
- **Durable binding (`resolve.rs`).** `SeedStore` persists at most one `SeedRecord` per operation, and
  `SeedSource` injects entropy so the draw count is observable. `resolve_seed` validates the setup
  first, then either restores an existing record or resolves a new one: a generate-once request draws
  at most once (a retry after a failed persist may redraw, since no record exists yet), an explicit
  request normalizes the supplied seed, and the record is persisted before the call returns. A persistence failure fails closed. A restored record must match the request's
  instance, baseline, lease, setup and mode, and its length-prefixed, mode-tagged binding digest must
  verify, or the request is refused. `requested_seed` and `effective_seed` stay distinct fields.
- **Start handoff (`resolve.rs`).** `dispatch_start` sends a `SentStart` carrying the persisted
  operation identity, effective seed and binding digest unchanged; `RecordingTransport` records what
  it was asked to send so the equality is testable.

## Consequences

- One logical run yields one effective seed once a record persists: a duplicate request, a lost
  reply and a restart all restore the persisted record without drawing again, because reuse never
  enters the draw path. Only a retry after a failed persist can redraw, before any record exists.
- Setup cannot be mutated on an unpersisted seed, because the record is written before resolution
  returns and a write failure is an error.
- A seed bound to the wrong instance, a stale baseline or lease, an unsupported setup, or a prior
  conflicting seed is refused before any draw or mutation.
- Cost: one new module and one versioned record schema. The alternative — resolving the seed inside
  the existing transport on each start — was rejected because the transport cannot distinguish a
  fresh attempt from a reconcile of the same attempt without exactly this persisted binding.

## Validation

- `crates/harness/tests/seed_binding.rs` covers an explicit seed that never draws, a generate-once
  seed that draws exactly once and is reused across a duplicate and a restart, a lost-response retry
  that does not redraw, a persist failure that fails closed and leaves no record, the four
  reject-before-draw mismatches, an explicit-versus-persisted conflict, a generate-once versus
  explicit-record conflict, the 64-byte two-byte and four-byte multibyte boundaries and their 65-byte
  refusals, empty/control/padded refusals, and a recording transport that sends the persisted fields
  unchanged.
- Source-only: no native run is exercised; the exact-host seed witness remains unverified until
  sts2-game-mod#79 records evidence.

## References

- Issue [#103](https://github.com/AI-Ascension/sts2-harness/issues/103): "Resolve and durably bind
  explicit or generate-once seeds in authored workflow setup".
- [ADR 0036](0036-seeded-run-transport-v1.md): the explicit seeded-run transport reused here.
- [sts2-game-mod#79](https://github.com/AI-Ascension/sts2-game-mod/issues/79): the native setup proof
  that stays external.
