# ADR 0069: Admit alternative gameplay forks from verified seeded replay prefixes

Status: accepted for the harness-owned, source-only slice of issue
[#117](https://github.com/AI-Ascension/sts2-harness/issues/117) — a validated prefix boundary, an
exact inherited binding, a zero-provider-call prefix replay, bounded sibling forks and a
replay-to-child handoff that reconciles a lost reply. It launches no game, replays no prefix and
invokes no provider: the replay port, the destination lease and the child process belong to the
gateway (sts2-gateway#50/#51), and native exact-host acceptance stays gated by sts2-game-mod#79. It is
ratified when the change carrying it merges.

## Context

Issue #117 requires an operator to restart one recorded seeded run, replay only a chosen settled
prefix and continue with a different decision in a fresh child branch, without an instant snapshot
restore and without claiming hidden-state equivalence. [ADR 0009](0009-seeded-episode-replay.md) and
`runtime_v3_episode_replay` already support `STS2_REPLAY_PREFIX=true`, public observation comparison,
legal-action rebinding and `PrefixVerified`, and the durable branch-store and reconciliation work
landed under #116. What was missing was the harness-owned contract that fixes which fork point may be
admitted and what a fork inherits, before any replay mutation or provider invocation.

Three failure modes had to be excluded by construction:

1. forking from a boundary that is not a settled, nonterminal decision with complete receipts;
2. letting a replay spend a provider call, diverge, or stop short yet still hand off a child;
3. letting two continuations of one prefix share an identity, or adopting a lost handoff reply twice.

## Decision

A new module `crates/harness/src/benchmark_manifest/prefix_fork/` owns that contract, split so each
file stays inside the production size budget, and is additive to the existing replay machinery:

- **Immutable binding (`binding.rs`).** `ForkBinding` binds the seed, launch profile, build digest,
  compatibility revision and the recorded prefix digest. `compare` lists every differing category in
  a stable order and `is_compatible` is exactly "no differing category", so no field can be widened
  to force equality.
- **Selected boundary and replay observation (`boundary.rs`).** `PrefixBoundary` names the settled
  occurrence, its decision ordinal, the captured state digest, terminality, the settled-action
  receipt list against an expected count, and one `LegalBinding` (a single resolved action key, or an
  explicit `Ambiguous`). `ReplayObservation` is `Verified` with its settled-action and provider-call
  counts, `Diverged` at an ordinal, or `Incomplete` against an expected count. Ordinals and receipts
  are bounded by `MAX_FORK_ORDINAL` and `MAX_PREFIX_RECEIPTS`.
- **Effect-free admission (`admission.rs`, `error.rs`).** `admit_prefix_fork` runs a fixed check order
  — labels, the boundary declaration, the requested-versus-recorded binding, terminality, settledness,
  receipt completeness, the legal binding, then the observed replay — so the first failing property is
  reported. A verified replay must carry exactly the boundary's settled-action count and zero provider
  calls; anything else returns the specific `PrefixForkRefusal` before the replay mutation or provider
  invocation it guards. No supplied value is reflected in a message.
- **Sibling forks (`sibling.rs`).** `SiblingSet` keeps one immutable source prefix and admits up to
  `MAX_SIBLINGS` continuations that each own a distinct trajectory, operation and context identity;
  a repeated identity, a foreign prefix and a duplicate all refuse.
- **Replay-to-child handoff (`handoff.rs`).** `admit_handoff` requires a retained destination to be
  revalidated and refuses a destroyed, lost or changed one; `next_handoff_stage` admits only the single
  forward successor of each stage and nothing out of the terminal stage; `reconcile_lost_handoff`
  adopts an already-admitted child and otherwise requires the prefix to be re-verified, so a lost
  reply never creates a second child or skips verification.

## Consequences

- A fork can only be admitted from a settled, nonterminal boundary whose receipts are complete and
  whose legal action resolves, because every other case refuses before any effect.
- A prefix replay that would spend a provider call, diverge from the recording or stop short cannot
  hand off a child, because the observation is part of admission rather than a post-hoc report.
- Two continuations of one boundary never share authority, and a lost handoff reply is idempotent.
- Cost: one new module and two small registries (labels, siblings). The alternative — forking directly
  from a checkpoint payload — was rejected because replay reconstructs the boundary by executing
  earlier actions and does not prove hidden-state equivalence, which is exactly the property #117
  preserves.

## Validation

- `crates/harness/tests/prefix_fork.rs` covers a verified prefix that replays exactly and is admitted,
  a stable binding-mismatch category order, a provider call during replay, a terminal or unsettled
  source, a missing receipt, an ambiguous legal binding, an observed divergence and an incomplete or
  short replay, invalid labels and out-of-range declarations, siblings that share one prefix with
  distinct identities, the sibling bound, retained/destroyed/lost/changed targets, a lost handoff
  reply, and a forward-only stage chain.
- Source-only: no native run, prefix replay or provider call is exercised; the exact-host fork witness
  and the real child-process handoff lane remain unverified until their gates record evidence.

## References

- Issue [#117](https://github.com/AI-Ascension/sts2-harness/issues/117): "Create alternative gameplay
  forks from verified seeded replay prefixes".
- [ADR 0009](0009-seeded-episode-replay.md): seeded episode replay and prefix verification reused here.
- [ADR 0068](0068-cold-launch-trial-isolation.md): the sibling per-trial isolation contract.
- [ADR 0021](0021-benchmark-manifest-foundation.md): private benchmark declarations.
