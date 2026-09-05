# Harness Architecture

Runtime-v3 recovery retains its operation ledger across at most two initialized replacement MCP
processes per episode. Replacement permits recovery reads only; it does not retry a mutation.
An uncertain allocation triggers configured fenced lease cleanup and reports cleanup failure.
Recovered receipts must match the full dispatched action identity and kind before settlement.

## Purpose and ownership

The harness is the coordinator, experiment, and artifact owner. Its intended scope includes up to
four allocated instances, provider decisions, trajectories, replay/evaluation, and artifact lineage.
The current Runtime-v3 executable coordinates one configured instance; record, memory, evaluation,
replay, artifact and co-op library seams are not all assembled into that executable. Ownership of a
responsibility is not evidence of a working multi-instance experiment. The harness does not become
a game adapter, gateway, MCP implementation, or provider implementation.

The target owner is the harness maintainers. The ownership/dependency decision is recorded in
[`decisions/0001-harness-ownership-and-dependency-boundary.md`](decisions/0001-harness-ownership-and-dependency-boundary.md);
the initial port surface is recorded in
[`decisions/0003-harness-initial-ports-and-deterministic-seams.md`](decisions/0003-harness-initial-ports-and-deterministic-seams.md).

## Runtime graph

```text
provider/model -- explicit provider port --> harness coordinator
harness -- MCP client --> MCP server -- protocol mapping --> gateway
gateway -- allocation, lease, fencing, routing --> game-mod/host instances
harness -- artifact port --> approved artifact or dataset store
```

The gateway owns instance lifecycle, readiness, routing, leases, and fencing. The MCP server is a
thin adapter that owns MCP framing and mapping. The game mod and host own authoritative state,
legality, mutation, host-thread affinity, and host lifecycle. The harness requests and observes
these boundaries; it does not implement a second authority.

Runtime communication and compile-time dependencies are separate. A future harness adapter may
depend on a declared gateway/MCP or provider port, but it must not depend on game-host, managed
loader, game-mod, or proprietary implementation crates. No harness component may contact a game
process directly.

## Compile-time direction

```text
record/protocol contracts <- coordinator policy <- boundary adapters
                                      ^                 ^
                                      |                 +-- MCP client port
                                      +-------------------- provider/artifact ports
```

This is a responsibility map, not a permission to create empty crates. The initial target-owned
`crates/harness` package provides pure ports and coordinator policy for the accepted preparation
surface. Further product modules should be introduced only when a real cohesive responsibility and
contract exist. Core coordination policy is testable without a socket, process, filesystem, clock,
provider, MCP runtime, or game.

The logical protocol layer is intentionally narrow, and its accepted implementation target is
`sts2-protocol`. It contains only language-/transport-neutral contracts that have a named owner,
producer, consumer, version, mapping, and conformance need. The target is a contract artifact/schema
repository, not a runtime service or a generic-common implementation crate. See
[`decisions/0002-sixth-target-protocol-decision.md`](decisions/0002-sixth-target-protocol-decision.md).

## Owned responsibilities

| Area | Harness ownership | Explicitly outside |
|---|---|---|
| Coordination | experiment/run/episode admission, four-instance allocation requests, synchronization, backpressure, cancellation, resumption | gateway allocator, game process lifecycle |
| Model boundary | provider-neutral port, prompts/inputs/outputs, budgets, execution identity, redaction | provider SDK internals and credentials |
| Records | observations, requested/accepted/completed action facts, events, snapshots, checkpoints, trajectories | authoritative game legality and host state |
| Replay/evaluation | deterministic replay inputs, divergence records, scoring/evaluator versions, comparisons | claim that replay proves a live game effect |
| Artifacts | dataset/model/evaluation lineage, manifests, hashes, retention and export policy | artifact-store infrastructure and private-data authorization |
| Protocol | harness consumes `sts2-protocol` profiles and owns only harness-specific trajectory/artifact shapes and mappings | protocol implementation internals, duplicate common crate, MCP framing, HTTP or host types |

## Concurrency and lifecycle

Typed decision replay lives in `decision_replay.rs`, separate from foundation record replay.
Evaluation aggregation and its report projection live in `evaluation.rs` and
`evaluation_report.rs`; the public replay and evaluation exports remain stable.

Every accepted operation resolves to success, explicit failure, or explicit cancellation. A returned
router binding with mismatched run or episode identity is unbound before admission fails; an unbind
failure remains visible as a routing error. Queues are
bounded and their overload behavior is part of the contract. Instance, gateway-session, MCP-session,
run, episode, model-execution, request, action, trajectory, and artifact lifecycles remain distinct.
Timeout or disconnect does not silently cancel work already accepted by a downstream boundary.

Ordering uses monotonic durations and explicit sequence/causality data; wall timestamps are for
observation only. Shutdown rejects new work, resolves owned queued work, closes ports, and joins
owned resources. Locking, retry, idempotency, stale lease, and duplicate-event behavior must be
specified before implementation.

## Security and data boundaries

Provider credentials originate outside ordinary trajectory records and are scoped, redacted, and
terminated at the appropriate port. Prompts, model outputs, game observations, saves, paths,
multiplayer identifiers, and artifacts are classified before storage or export. Cross-instance
correlation is explicit, and artifact lineage never grants authority to mutate a game.

## Evidence status

The architecture remains a proposed foundation contract, with confirmed deterministic, component,
and one bounded exact-host runtime lane. Deterministic fake tests exercise the local correlation,
retry/idempotency, record/replay, artifact-lineage, and cleanup seams; the dated component and host
records exercise real harness/MCP/gateway processes. No provider call, gameplay-rule mutation, score,
or live artifact publication has been executed by this repository.

The minimal POC adds a deterministic fake runner that consumes the copied `poc-v1` release-like
artifact and records the exact boundary order `harness -> MCP -> gateway -> game-mod -> game-core`.
It exercises one state read, one accepted typed action, and one rejected typed action through local
doubles only. The resulting trace and evidence labels are in
[`../MINIMAL_POC_REPORT.md`](../MINIMAL_POC_REPORT.md); they are `confirmed` deterministic-fake
and `source-derived` offline evidence, not runtime or compatibility evidence.

## Runtime coordinator adapter

ADR 0005 adds a standalone runtime binary under `crates/harness/src/bin/`. It is an explicit
coordinator, not a second MCP or gateway implementation. The binary uses a bounded HTTP client for
the gateway control call, launches the configured MCP process with stdin/stdout pipes, and keeps
gateway session, MCP session, lease, epoch, request, and action identities distinct.

The trace order is allocation, MCP initialize, catalog verification, state at generation N, action
at N, stale action at N, fresh state at N+1, and release. The coordinator requires an accepted
action's fresh `status_overlay_visible` witness and stable stale-generation error before emitting
the sanitized trace. It never contacts a game process directly. Process launch, graceful shutdown,
and the exact host effect are evidenced separately in the dated host integration record; gameplay
mutation and broader compatibility remain outside that record. The runtime-v2 profile propagates
the independently configured MCP session through allocation metadata, the spawned MCP process, and
lease release while retaining the gateway session as the protocol-envelope identity.

The pure Runtime-v2 coordinator seam additionally owns bounded admission for up to four registered
instance lineages. It keeps process-port, gateway-session, MCP-session, lease, run, episode,
trajectory, request, operation, trace, and artifact identities explicit; dispatches one action at a
time per instance; schedules ready instances round-robin; rejects global/per-instance overload; and
reports queued cancellation separately from active work requiring downstream reconciliation. This is
harness component policy, not a gateway supervisor or host lifecycle implementation.

An allocation failure or rejected allocation response triggers one fenced release attempt before
returning the original error. Cleanup may use a validated returned lease and epoch only when the
instance, caller, and gateway session match the allocation request. Otherwise it keeps the configured
fence. This cleanup does not replace the configured fence used for trace admission, and an unconfirmed
or failed release remains an explicit error. This behavior is source-derived; host cleanup is unverified.

## Runtime-v3 gameplay and Exo boundary

The runtime-v3 gameplay surface is split into two harness-owned layers. `episode/` requires a fresh,
actionable observation and the matching host-generated legal-action set before it asks a provider for
one choice. `exo/` sends only the sanitized fair-play projection, generation, objective, hard
constraints, and action IDs through an operator-supplied transport with a pinned revision and bounded
request/response/timeout settings. There is no heuristic action path when Exo is unavailable,
malformed, stale, or closed.

The projection root admits `state_id`, `generation`, `player`, `state`, `legal_actions`, and an
optional `visible_seed`. Because it is `unverified` whether the host's `visible_seed` is the real
PRNG seed or can be expanded into unrevealed outcomes, the seed is removed from every Exo request by
default (`ExoConfig::forward_visible_seed = false`, `SanitizedObservation::without_visible_seed`).
Only the explicit opt-in, `with_visible_seed_forwarding(true)` or the runtime environment value
`STS2_EXO_FORWARD_VISIBLE_SEED=true`, re-admits it; the gate applies to both the session and the
`ProviderPort` prompt path and is `confirmed` by the `provider_redaction` tests. See ADR 0006,
amendment 2026-09-05.

Accepted mutation is not settlement. `ActionLedger` records operation identity, the stability barrier
waits for a semantic successor or same-state mutation, and `verify_settlement` requires a fresh
observation plus an independent effect witness. Unknown outcomes enter explicit recovery/reconcile
operations; they are never retried as a new strategic action. The `experiments/exo-agent` directory
contains only an example configuration and boundary documentation. Exo connectivity, target-build
compatibility, and live gameplay remain `unverified` until a separate runtime handoff.

The episode surface routes the declared playable stages through the provider port, while terminal
and recovery states are handled by the episode state machine. Its full-run tests use scripted
observations and successors; they do not establish coverage of every target-game state or a live run.

The current [Runtime-v3 entry point](../crates/harness/src/bin/runtime_support/runtime_v3.rs)
assembles `EpisodeRunner`, one gateway/MCP port and an Exo decision source. It retains an in-memory
operation ledger and emits a terminal summary. It does not wire `DecisionRecord`, `DecisionMemory`,
`Evaluator`, `DecisionReplay`, artifact publication, or `CoopCoordinator` into this run. Their
separate library tests are not runtime trajectory, evaluation, persistence, or co-op evidence.

`CoopCoordinator` is a library gate for a caller-supplied generation snapshot. It checks registered
peers' reported generations, connection flags and ally targets but cannot perform a host mutation.
It has no expected-roster/quorum contract or coordinator-generation advancement API: an absent,
never-registered peer is not detected, and registering only the local peer can pass local admission.
Continuous co-op coordination requires an explicit authoritative generation/roster contract and
runtime wiring; it cannot be inferred from this gate. Decision records separately distinguish
requested, accepted, settled, recovery, estimate, and unavailable evidence. M10 patch manifests keep
build/data/UI/action/schema drift quarantined until exact runtime evidence exists.

`EpisodeRunner` is the bounded complete-run coordinator over an explicit `EpisodeRuntimePort`. Its
port is assembled by the gateway/MCP integration and exposes launch, observe, host-generated legal
actions, semantic dispatch, transition waiting, safe recovery, and ordered cleanup. The runner does
not own a game handle, choose a fallback action, or infer settlement from an acknowledgement. The
operator-owned `ExoProcessTransport` lives at the harness adapter boundary, outside `episode/` and
`exo/` core policy: it directly invokes a configured bridge with bounded stdin/stdout, timeout, and
environment allowlisting. The existing core CI guard therefore continues to reject process or
socket access inside the fair-play/provider policy module.

The process adapter uses a joined supervisor and cancellable asynchronous pipes; no detached
blocking readers or writers survive an exchange deadline. Direct-child kill/reap has a separate
250-ms cleanup grace period. Descendant process termination and OS sandboxing are outside this
adapter's guarantee; operator containment is still required. See ADR 0006 for the dependency and
failure-handling rationale.
