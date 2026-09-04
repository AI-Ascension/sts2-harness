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

## Evidence

Deterministic routing, sandbox, record, replay, evaluation, and manifest checks are source-derived or
confirmed at their stated layer. Exo connectivity, licensed-host progression, co-op traces, and
release promotion are unverified or quarantined.
