# Seeded episode replay

Status: accepted implementation boundary; complete live campaign replay remains unverified.

The runtime-v3 episode runner previously selected the Exo provider even when an operator supplied
`STS2_REPLAY_TRAJECTORY`. Only the separate combat demonstration consumed that setting. The full
episode path now selects a replay decision source before constructing any provider transport.

The source must begin at an explicitly seeded setup, contain a bounded sequence of decisions with
matching operation receipts and settlement observations, and end at victory or defeat with the same
seed. Validation rejects incomplete, resumed, conflicting, unsupported, and oversized sources before
the episode runner allocates a lease. Operation identities cannot be reused between decisions.

A first receipt reporting `Rejected` or `StaleState` may be omitted from replay dispatch only when
it has no effect and its public observation matches the decision observation under the comparison
rules below. The source digest and skipped-attempt count preserve this provenance. A rejection
after an `Unknown` receipt remains invalid; it cannot prove that the earlier mutation did not occur.

Before every replay action, the current public observation must match recorded gameplay content.
Only generation, state identity, and the legal-action catalog are excluded from that comparison;
the selected recorded action payload must separately identify exactly one current legal action.
The runner dispatches that current identity through its existing MCP and gateway ports. Replay never
restores a save, changes host state directly, or falls back to a provider on divergence.

The replay cursor advances only after settlement. Terminal verification requires every recorded
action to settle and the terminal public observation to match. `replay_decision` records have no
model execution identity. `episode_replay_verified` records the source digest and zero provider calls.

An operator may explicitly set `STS2_REPLAY_PREFIX=true` for a source truncated at a settled,
actionable, nonterminal checkpoint. This mode rejects terminal sources and unresolved trailing
actions. After matching the checkpoint, it requests the existing stop-episode recovery operation,
which releases the lease and performs owned runner cleanup. Only successful cleanup yields
`episode_replay_prefix_verified`; it does not emit episode completion or claim full-run replay.
Any later provider continuation is a separate invocation and evidence segment.

Deterministic source tests cover matching semantics with fresh identities, divergent seed/player/
action content, ambiguous current actions, unresolved operations, invalid source lineage, terminal
comparison, and explicit prefix admission. These tests are synthetic coordination evidence.

The configurable episode step ceiling increases from 1,024 to 4,096. A real campaign consumes
steps for idle transitions and reconciliation as well as decisions, so a complete replay can
exceed the old ceiling. The budget remains explicit and bounded; runtime defaults are unchanged.
A synthetic 1,100-transition episode verifies completion and owned cleanup above the old limit,
while zero and values above the new ceiling remain invalid.
