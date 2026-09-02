# Product Contract

## Purpose

`sts2-harness` is a Rust-first experiment and training harness for coordinating model-driven STS2
experiments through declared MCP and gateway interfaces. It owns experiment control, reproducible
records, replay/evaluation, and artifact lineage while remaining outside the game process.

The target owner is the harness maintainers. Public behavior becomes a product contract only when a
project-owned requirement, acceptance test, and compatibility classification are approved. Wave 2
implements only the preparation seams described below; it does not claim live integration behavior.

## Initial scope

The eventual product may own:

- experiments, runs, episodes, bounded coordination, and up to four instance assignments;
- MCP-client and gateway-control-plane ports, without bypassing their authorities;
- provider-neutral model execution, prompts, input/output budgets, cancellation, and rate limits;
- observation/action correlation, append-only trajectory events, snapshots, and checkpoints;
- deterministic offline replay, divergence detection, scoring, evaluation, and evaluator versions;
- dataset export, optional fine-tuning/training integration, and model/evaluation artifacts;
- retention, redaction, reproducibility, resumption, operator controls, and audit records; and
- harness-owned schemas, manifests, compatibility records, and release evidence.

The Wave 2 package implements the narrow identity, routing, provider, record, replay, artifact, and
shutdown seams with bounded values and deterministic fake tests. Scoring, evaluation policy, dataset
export, training, and external adapters remain planned responsibilities rather than implemented
product integrations.

## Non-goals

The harness will not own game host objects, loader metadata, managed assemblies, game UI, saves,
direct game-process access, game rules, state extraction, action legality, host mutation, or a second
game adapter. It will not own gateway instance lifecycle, leases, fencing, or routing internals; MCP
framing and tool semantics stay with the MCP server; provider SDKs and credentials stay outside the
provider-neutral port.

It will not infer correctness from a model response, accepted action, reachable process, recorded
trajectory, successful serialization, or score. It will not export private prompts, model output,
saves, paths, multiplayer data, or credentials by default. It will not add hidden network discovery,
unbounded payloads, implicit trust, or a generic-common protocol implementation. The accepted
`sts2-protocol` target owns its narrow shared contract artifacts; the harness does not duplicate them.

## Contract development

Before adding a route, field, action reference, event, error, lifecycle state, provider operation,
replay rule, score, artifact, or CLI surface, define its owner, namespace, version, bounds,
optionality, ordering, error behavior, security impact, provenance, and deterministic acceptance test.
Keep `instance_id`, `session_id`, `run_id`, `episode_id`, `trajectory_id`, `request_id`, `action_id`,
`trace_id`, `model_execution_id`, and `artifact_id` distinct. Record mappings rather than reusing a
generic identifier.

## Language and boundary rule

Product logic, contract handling, repository tools, generators, and tests are Rust. Core policy is
free of transports, hosts, processes, and concrete providers. A future external adapter may translate
to MCP, gateway, provider, or artifact-store behavior through an explicit port. Managed or proprietary
host integration is not a harness exception because it belongs to another repository.

## Quality gates

The repository policy checker enforces required foundation files, Markdown links, workflow pins,
language restrictions, MIT headers, and bounded files. Rust formatting, Clippy, and tests are required
for Rust changes. Runtime/provider/game compatibility advances only with exact controlled evidence;
the foundation phase remains runtime-unverified.
