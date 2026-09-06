# Live episode stability candidate

Status: experimental, incomplete, unmerged. The existing full-run coordinator remains
separate from proof that a real host implements every campaign surface.

When an observation is temporarily blocked, the runner waits through its bounded stability
port before consulting policy. Accepted or unknown actions reconcile and wait under the
same operation identity; they are never replaced with a newly dispatched action. A settled
transition still requires the host's effect witness and independent validation.

The MCP transport implements bounded timed polling because the current native wait route
returns immediately. Idle waits read authoritative observations; they do not create an
action-settlement witness. Diagnostics distinguish timeout from other barrier failures.

`STS2_LIVE_EPISODE=true` enables the explicit OpenAI Astra lane and emits decision, receipt,
operation-wait, and terminal records. The executable SHA256 remains verified. Other provider
kinds are rejected in live episode mode. Records are experimental and do not constitute a
complete replay format or full-run proof.

Confirmed source validation: workspace Clippy and tests pass. Episode tests cover blocked
readiness, timeout cleanup, delayed same-operation completion, and unresolved mutation
without duplicate policy or dispatch calls.

Confirmed host diagnosis: Windows readiness changed while the old mod generation stayed
constant. The harness timed out. The paired mod candidate adds the omitted internal
readiness fields and legal catalog to its generation fingerprint. Another queued-map
attempt returned Recovery; independent postcondition validation rejected it.

Unverified: uninterrupted full campaign, campaign replay, Linux gameplay, and the final
paired host candidate. Do not merge based only on synthetic tests or partial trajectories.
