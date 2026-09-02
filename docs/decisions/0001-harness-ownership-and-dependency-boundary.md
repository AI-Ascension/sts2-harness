# ADR 0001: Harness Ownership and Dependency Boundary

## Status

Accepted for the Wave 1 foundation and retained for Wave 2. Runtime integrations remain unapproved
until their contracts and tests are added.

## Context

The STS2 system has separate concerns for host integration, gateway control, MCP translation, and
experiment/model work. A harness that reaches directly into a game process would duplicate authority,
make four-instance isolation ambiguous, and make model or replay evidence impossible to interpret.

The harness also has a different lifecycle from a game instance. Experiments, runs, episodes, model
executions, trajectories, scores, datasets, and artifacts must survive or fail independently of an
individual gateway lease or provider request.

## Decision

`sts2-harness` owns:

- experiment, run, episode, and model-execution coordination;
- bounded allocation requests for up to four game instances, synchronization, backpressure,
  cancellation, resumption, and operator control;
- provider-neutral model ports, prompts/inputs/outputs, budgets, rate limits, and provider-result
  correlation, without owning provider SDKs or credentials;
- observation and action records, trajectory events, snapshots, checkpoints, deterministic replay,
  divergence reporting, scoring, evaluation, and evaluator versioning;
- dataset preparation, optional training/fine-tuning integration, artifact manifests, hashes,
  retention, redaction, reproducibility, and lineage; and
- harness-owned record schemas, conformance fixtures, compatibility statements, and release evidence.

The other boundaries retain these authorities:

| Boundary | Authority |
|---|---|
| Game host / game mod | host objects, authoritative state, rules, legal actions, mutation, and host-thread access |
| Gateway | game-process lifecycle, allocation, readiness, routing, leases, isolation, and fencing |
| MCP server | MCP framing, tool contract, session mapping, and MCP-to-gateway translation |
| `sts2-protocol` | narrow language-/transport-neutral shared schemas, version/profile descriptors, manifests, and conformance vectors; no generic-common implementation |
| Game/core | host-independent semantic/domain policy and its own owned contract meanings |
| Provider | model service implementation, provider credentials, and provider-side execution |

The harness consumes MCP and gateway behavior through declared ports. It may use a direct gateway
control-plane port only for explicitly gateway-owned operations; it never contacts a game process or
game-mod listener directly. The MCP server remains a thin adapter. Core coordination policy remains
free of transports, host objects, processes, and concrete providers.

## Dependency direction

```text
sts2-protocol contract artifacts -> record mappings <- pure coordinator policy <- boundary adapters
```

This compile-time graph is separate from the runtime graph. The accepted protocol target distributes
only language-/transport-neutral artifacts and conformance inputs; it has no runtime authority and no
dependency on consumer implementation crates. No harness crate or module may depend on game-host,
managed-loader, game-mod implementation, or proprietary host code. A runtime message does not grant
permission to import a peer's private implementation types. Additional product crates are not
created until a real responsibility, owner, contract, and test suite exists.

## Contract and evidence consequences

Every boundary record keeps `instance_id`, `session_id`, `run_id`, `episode_id`, `trajectory_id`,
`request_id`, `action_id`, `trace_id`, `model_execution_id`, and `artifact_id` in distinct namespaces.
Versions for harness, trajectory/schema, scoring, training/dataset, provider profile, MCP, gateway,
game-mod, host, and runtime remain independent. A trajectory preserves observation, requested action,
acceptance, completion, score, and provider facts separately.

`confirmed`, `source-derived`, `inferred`, `proposed`, `unverified`, and `unsupported` are distinct
evidence states. A model response, accepted action, acknowledgement, handshake, or trajectory does
not prove game effect, semantic correctness, replay fidelity, or compatibility.

## Alternatives considered

1. **Direct game access from the harness:** rejected because it bypasses host and gateway authority.
2. **A harness-owned gateway or MCP implementation:** rejected because it creates competing lifecycle
   and protocol authorities.
3. **Provider SDKs in coordinator policy:** rejected because it couples experiments to credentials,
   network behavior, and provider-specific failures.
4. **One broad common crate:** rejected because it hides ownership and would make transport/host
   dependencies leak inward.

## Revisit conditions

Revisit this decision if a boundary requires a separately owned lifecycle, a new trusted authority,
an independently released contract, or evidence that the current ports cannot express safe lifecycle,
privacy, or compatibility behavior. Any change needs an ADR, dependency-graph review, migration plan,
and updated deterministic conformance cases.
