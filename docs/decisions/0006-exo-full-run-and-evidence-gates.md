# ADR 0006: Exo full-run coordination and evidence gates

- Status: Accepted as a bounded harness design; live provider and target runtime remain unverified
- Date: 2026-09-04

## Context

Autonomous control must cover every run surface, but deterministic code cannot become a gameplay
policy fallback. Exo is external and must receive only fair-play data. Accepted actions require a
fresh transition or explicit recovery, and release evidence must separate source, fake, host, and
end-to-end claims.

## Decision

Keep Exo behind the provider port with a pinned operator-supplied revision, bounded I/O, strict
structured decisions, current host action binding, and no heuristic action on unavailability. The
episode router sends setup, map, combat, reward, shop, event, rest, and selection choices through
the provider; terminal victory/defeat and recovery are explicit state-machine outcomes. Typed
records distinguish observation, request, acceptance, settlement, recovery, estimate, and
unavailable evidence. Replay/evaluation consume bounded records but cannot grant host authority.

The research field package is classification metadata only. M10 manifests record build, data, UI,
action, and schema drift independently and remain quarantined until exact hashes, leak checks,
clean-install/replay, rollback, cleanup, and live traces exist.

The harness exposes a bounded `EpisodeRunner` over an explicit runtime port. The runtime port must
provide launch, observation, the matching host-generated legal-action set, semantic dispatch,
transition waiting, safe recovery, and ordered shutdown. Every model-selected action is rebound to
the current catalog before dispatch; an accepted or unknown mutation is not retried strategically.
The runner reconciles uncertain outcomes or stops fail-closed, and cleanup attempts lease release,
MCP close, and gateway close independently.

Because the public Exo API is low-level and deployment-specific, the harness also provides an
operator-owned `ExoProcessTransport`. It sends one sanitized JSON request to a directly invoked
bridge process and accepts one bounded JSON response. The process receives only an explicit
environment-name allowlist, and the harness never invokes a shell. This is an adapter seam, not a
claim that Exo or a licensed STS2 build is connected.

### Process cancellation correction

The original blocking pipe workers could outlive an exchange when a descendant inherited stdin
or stdout. Killing the direct child does not close the descendant's pipe handles. A local Linux
probe reproduced eight stranded workers after four timeouts; the same regression now requires
the harness thread count to return to baseline before the fixture descendants exit.

Use pinned Tokio 1.53.1 only in the process adapter, with `rt`, `process`, `io-util`, `time`, and
`macros` features. One scoped, joined supervisor owns a current-thread runtime and concurrently
polls writes, bounded reads, and child exit. No detached pipe tasks or blocking I/O workers are
created. A single exchange deadline cancels both pipe futures and closes their owned handles.
An error terminates the direct child and gives reaping a separate 250-ms grace period; failed
cleanup reports `Unavailable`. The scoped thread is joined before return. This remains a
synchronous port and blocks its calling thread even when called from an async runtime.

This dependency replaces uncancellable blocking I/O without adding local unsafe code or OS-specific
FFI. It does not add a provider SDK, change wire contracts, or move process access into core policy.
Tokio's [child lifecycle](https://docs.rs/tokio/1.53.1/tokio/process/struct.Child.html) supplies
cancellable wait and explicit kill/reap operations; kill-on-drop is a fallback, not a proof of reaping.
OS process creation and scheduling are not hard-real-time guarantees. Kernel-stalled termination
may exceed successful-cleanup guarantees and must be reported as unavailable, not clean shutdown.

Only the directly spawned bridge is terminated. Descendants remain the operator's containment
responsibility; the transport is not an OS sandbox. Linux subprocess regressions are checked in
CI; Windows/macOS process behavior and arbitrary descendant termination remain unverified.

## Evidence

The patch-diff preparation utility is a workspace member so the same format, lint, and test gates
cover it. It uses the existing pinned Serde dependencies to parse bounded JSON and reads the declared
quarantine status structurally; FNV fingerprints remain non-cryptographic line-diff aids. Its
test-only `jsonschema` 0.52.1 dependency, with default URL/file resolution features disabled, validates
the canonical manifest against the full Draft 2020-12 schema. Negative vectors exercise nested
constraints, and a separate assertion keeps the checked-in preparation manifest quarantined.
Neither parsing nor schema conformance establishes artifact integrity, runtime truth, or promotion
authority. This dependency does not enter the provider or episode runtime path.

Deterministic routing, sandbox, record, replay, evaluation, and manifest checks are source-derived or
confirmed at their stated layer. Exo connectivity, licensed-host progression, co-op traces, and
release promotion are unverified or quarantined.

## Amendment 2026-09-05: `visible_seed` fails closed

Review finding L2-P10-1 (Wave 1, 2026-09-04): `visible_seed` sat in the root allowlist of
`exo/sandbox.rs` and was forwarded verbatim to Exo, while `experiments/exo-agent/README.md` states
that Exo must not receive future RNG state. The only rationale was the `context-only` /
"opaque visible text with no RNG expansion" row in
`docs/evidence/runtime-v3-preparation/data/information-importance.csv`. Whether the host's
`visible_seed` is the real PRNG seed, or can be expanded into unrevealed outcomes, depends on the
MCP/host producer and is `unverified`.

Decision (delegated by the owner to L0 on 2026-09-05): fail closed.

- The default fair-play projection sent to Exo omits `visible_seed`. `ExoConfig` gains
  `forward_visible_seed` (default `false`) and `with_visible_seed_forwarding(bool)`;
  `SanitizedObservation::without_visible_seed` removes the key. The gate is applied in both
  request paths, `ExoSession::decide` and the `ProviderPort` structured-prompt path, before any
  transport is reached.
- The runtime binary reads `STS2_EXO_FORWARD_VISIBLE_SEED`; only the exact strings `true` or
  `false` are accepted, absence means `false`, and anything else is a configuration error.
- At the projection root `visible_seed` is now optional; `state_id`, `generation`, `player`,
  `state`, and `legal_actions` remain required and no other root key is admitted. The host-facing
  `runtime-v3-gameplay` parser and schema still require the field from the host; no protocol
  artifact, schema, golden, or checksum inventory changed.

Labels: the gate (absent by default, present only when enabled, on both paths) is `confirmed` by
`crates/harness/tests/provider_redaction.rs` and `fair_play_firewall.rs`; the seed-is-PRNG
question stays `unverified`. An operator who enables forwarding accepts that fair-play risk and
must record the rationale in the run's handoff.

## Owner correction 2026-09-05: repeatable seed invocation and replay

The owner clarified that runs must be callable repeatedly with the same seed and replayable;
seed exposure must not be blocked by default. This supersedes the default-off decision above.
`ExoConfig::new` and an absent `STS2_EXO_FORWARD_VISIBLE_SEED` now preserve the host-visible seed.
An explicit `false` remains available for seed-blind experiments. Both request paths have
regressions for default seed preservation and explicit omission. Hidden RNG internals remain
outside the observation contract. Reproducibility requires recording the seed alongside host,
mod, protocol, model, and action lineage; a seed alone does not prove identical model decisions.
