# ADR 0074: Wait for an identity-bound readiness milestone

Status: accepted for the harness-owned, source-only slice of issue
[#96](https://github.com/AI-Ascension/sts2-harness/issues/96) — a versioned readiness target and a
bounded, per-generation wait that only a fresh authoritative observation can settle. It launches no
process, reads no gameplay state and spends no provider call; the Studio round-trip and the native
loading verification are separate slices, and native mapping stays gated by the separately
authorized game/gateway lanes. It is ratified when the change carrying it merges.

## Context

Issue #96 requires an authored workflow to advance when a specific authoritative readiness milestone
is reached, rather than on a fixed sleep or on a process answering on a listening port. Harness
already separates a launch acknowledgement from gameplay readiness and binds readiness evidence to
an instance and authority epoch (`management/lifecycle_readiness.rs`), but that split cannot *wait*:
it names no milestones, carries no versioned target or bounded deadline, and does not bind a
generation or invalidate on restart.

Four failure modes had to be excluded by construction:

1. an unsupported target being silently accepted instead of refused before work starts;
2. a foreign or stale observation satisfying a wait for another instance, epoch or generation;
3. an unbounded wait with no distinguishable timeout, denial or cancellation outcome;
4. a restart leaving prior readiness usable, so cached pre-restart state settles a later wait.

## Decision

A new module `crates/harness/src/management/readiness_wait/` owns the contract, split so each file
stays inside the production size budget, and is additive to `lifecycle_readiness`:

- **Vocabulary (`milestone.rs`).** `ReadinessMilestone` is an ordered, stable-labelled set —
  `Booted`, `AdapterCompatible`, `LeaseInstalled`, `SetupAvailable`, `Actionable` — reported by the
  observing owner. A milestone is never inferred from elapsed time or from a listening port.
- **Versioned target (`target.rs`).** `ReadinessTarget` carries the target milestone, a bounded
  `deadline_ms`, a `max_attempts` budget and a `contract_version`. It deserializes with
  `deny_unknown_fields`, so an unknown member is refused at decode time, and `new` fails closed with
  `Incompatible` for an unsupported version and `InvalidTarget` for a zero deadline or attempt bound.
- **Bounded wait (`wait.rs`, `error.rs`).** `ReadinessWait::begin` binds one instance, authority
  epoch and process generation. `observe` accepts an existing sealed
  `GameplayReadinessEvidence` plus the owner-reported milestone and generation: foreign instance or
  epoch yields `ForeignReadiness`, a superseded generation yields `StaleReadiness` without spending
  the budget, and an admitted observation below the target returns `AwaitingMore` without settling.
  `deny`, `cancel` and `invalidate_for_restart` each settle the wait once into a distinct
  `ReadinessTerminal`, and the deadline or attempt budget yields `Timeout`.

## Consequences

- A wait settles only from fresh, correctly bound, at-or-above-target evidence; a launch
  acknowledgement still cannot be substituted because it is not readiness evidence.
- The refusal vocabulary separates timeout, denial, cancellation, restart invalidation and
  incompatibility for explicit workflow routing, and an unsupported target is refused before any
  work starts.
- This slice maps no live gateway/mod evidence and performs no native loading. Studio capability
  fields and the native check that a listening port cannot satisfy gameplay readiness (issue #96
  T3) remain open and are not claimed satisfied here.
