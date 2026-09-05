# Harness Architecture

## Purpose and ownership

The harness is the coordinator, experiment, and artifact owner. It coordinates bounded runs across
up to four allocated game instances, mediates model/provider executions, records episodes and
trajectories, supports replay and scoring, and preserves lineage for datasets and model artifacts.
It does not become a game adapter, gateway, MCP implementation, or provider implementation.

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
