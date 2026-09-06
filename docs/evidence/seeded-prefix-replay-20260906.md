# Confirmed seeded prefix replay, 2026-09-06

An authorized fresh Linux v0.107.1 fixture replayed 384 recorded actions from seeded setup to
the Act 2 Crystal Sphere event, with seed `AIASCENSIONV3FULL1`. The runtime selected replay
before constructing a provider transport. Every recorded action matched current public gameplay
content and a unique current legal action, then settled through the MCP/gateway/game-mod path.
The checkpoint matched, owned cleanup succeeded, and the harness exited zero.

The source is an explicitly composed prefix of three settled Astra trajectory segments. It is
not an uninterrupted provider run. Composition stops before the unresolved Crystal Sphere action
from the earlier host; that unresolved mutation is neither retried nor included in this source.
One initial rejected admission with unchanged public state was counted and skipped. The source
therefore contains 385 decision attempts and 384 replayed actions.

| Evidence | Value |
| --- | --- |
| Composed source SHA-256 | `ebbf3d4e2f872270743608ceb5a8ac60193f3ecbd786e0006bae37edc19788f0` |
| Fresh replay trajectory SHA-256 | `c59a04d0be3f4a9f927a90b363316b639a3208b2d09cb288e04393e6997fd052` |
| Linux addon SHA-256 | `c16946495ab0b8b8cf4cf3cd44d9a666be8893c5b88380f40f484462cfc9c3c9` |
| Replayed actions | 384 |
| Skipped rejected attempts | 1 |
| Provider calls | 0 |
| Final record | `episode_replay_prefix_verified` |

The game remained visible in its read-only Linux video window; a fresh capture was required
before starting replay. No OS keyboard/mouse automation or save restoration drove the replay.
Private trajectory and composition artifacts remain outside the repository.

This confirms prefix fidelity and cleanup, including passage through the earlier combat-to-reward
boundary. It does not establish setup-to-victory/defeat replay, completion of Crystal Sphere, a
full Astra campaign, or Windows full-campaign fidelity. Any provider continuation is a separate
invocation with separate evidence. Complete campaign replay remains unverified.
