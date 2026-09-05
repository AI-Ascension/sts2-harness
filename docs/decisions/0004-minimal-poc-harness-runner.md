# ADR 0004: Minimal POC harness runner

- Status: Accepted for deterministic POC evidence
- Date: 2026-09-02

## Context

The harness is the coordinator and evidence owner for one narrow proof-of-concept slice. The slice
needs to demonstrate a protocol-bound state read, one accepted typed action, one rejected typed
action, and explicit provenance across the requested boundary order without requiring a live game or
network access.

## Decision

The harness consumes the copied release-like `sts2-protocol/poc-v1` artifact under
`protocol-artifact/poc-v1/`. A fixed seed (`7`), fixed clock tick (`0`), instance, session, and lease
drive local doubles in this order:

```text
harness -> MCP -> gateway -> game-mod -> game-core
```

The runner performs a state read, an accepted `use_budget` action of one unit, and a rejected
`use_budget` action of zero units. It emits one canonical trace event at each of the five boundary
labels for each operation. Every event carries protocol version, schema digest, artifact/source/
generator provenance, correlation, instance, lease, generation, bounded observation, typed action,
status, and error fields. The report labels evidence as `confirmed` (deterministic fake only),
`source-derived`, `proposed`, or `unverified`.

## Consequences

The result is deterministic, reviewable, and runnable offline with no cross-repository implementation
dependency. It proves only local fake composition and the stated state-transition assertions. Live
MCP/gateway transport, process and host loading, game compatibility, action legality, effect settlement,
provider execution, and runtime cleanup remain unverified.
