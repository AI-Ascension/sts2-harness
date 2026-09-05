# Runtime-v3 live combat demonstration

`STS2_COMBAT_DEMO=true` selects a bounded combat loop through the normal MCP/gateway port.
The mod owner launches an isolated host and bootstraps the combat. The harness never accesses
the host process or saves. Every selected action binds to the fresh host catalog; accepted or
unknown outcomes reconcile under the same operation ID. An MCP error flag on a gameplay
receipt does not bypass receipt validation or make that receipt a transport failure.

Build `sts2-harness-runtime` and `sts2-ollama-bridge` with locked Cargo dependencies.
The bridge uses a bounded HTTP request to the local Ollama endpoint on port 11434 and model
`gemma4:31b-cloud`. It uses the existing structured Exo transport seam; it does not run Exo.
Set `STS2_PROVIDER_KIND=ollama`, `STS2_EXO_BRIDGE_BINARY` to its executable, and
`STS2_EXO_REVISION` to that executable's SHA-256. Arguments must be empty. The harness
verifies the digest before starting. Normal Exo runs retain their reviewed revision gate.

Visible seeds are forwarded by default. Set `STS2_EXO_FORWARD_VISIBLE_SEED=false` only
for an intentional seed-blind experiment. The bridge accepts one current legal action and
a short rationale; it has no heuristic fallback. Store trajectories only in an operator-owned
external directory because they contain visible game data and model rationales.

Set `STS2_REPLAY_TRAJECTORY` to an external completed model trajectory for action replay.
This mode makes no provider calls, checks each visible pre-action observation including seed,
resolves recorded semantic choices against the fresh host catalog, and verifies terminal state.
Generation/state IDs are local to each process and are normalized for comparison. A changed
seed or other visible divergence fails before dispatch. Replay events are labeled separately.

Confirmed live on 2026-09-05: the Rust bridge selected 18 real actions, all settled with host
witnesses, and reached Reward. Two fresh-process replays repeated those choices, including
a fullscreen run that checked terminal state. A different seed failed before the first action.
A combat result does not prove a full seeded campaign.
