# Repository Instructions for Coding Agents

## Scope and authority

These instructions apply to the `sts2-harness` repository. Follow direct user instructions first,
then this file, then the canonical documents below:

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- [`docs/PRODUCT.md`](docs/PRODUCT.md)
- [`docs/CODING_STANDARDS.md`](docs/CODING_STANDARDS.md)
- [`docs/TESTING.md`](docs/TESTING.md)
- [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md)
- [`docs/LICENSING.md`](docs/LICENSING.md)
- [`docs/WORKFLOWS.md`](docs/WORKFLOWS.md)
- [`docs/POLICY_AS_CODE.md`](docs/POLICY_AS_CODE.md)
- [`RELEASING.md`](RELEASING.md)

The ownership and dependency decisions are recorded in
[`docs/decisions/0001-harness-ownership-and-dependency-boundary.md`](docs/decisions/0001-harness-ownership-and-dependency-boundary.md)
and [`docs/decisions/0002-sixth-target-protocol-decision.md`](docs/decisions/0002-sixth-target-protocol-decision.md).
The initial target-owned port surface is recorded in
[`docs/decisions/0003-harness-initial-ports-and-deterministic-seams.md`](docs/decisions/0003-harness-initial-ports-and-deterministic-seams.md).

## Target contract

The target owner is the harness maintainers. This repository is the coordinator and experiment /
artifact owner for multi-instance runs, model/provider ports, episodes, trajectories, replay,
scoring, evaluation, datasets, and artifact lineage. It is not a game adapter.

Wave 2 permits one non-empty target-owned Rust harness package with explicit ports and deterministic
fake-boundary tests. Keep it to coordination seams, identity, records, replay checks, artifact
lineage, and lifecycle cleanup; do not add live provider behavior, game behavior, gateway lease
ownership, MCP framing, model weights, datasets, or copied implementation source. The existing
directory and any future `experiments/managed-rust-interop` work must remain intact and must not be
treated as game authority.

## Non-negotiable boundaries

- The harness has no direct game-process, host-object, loader, managed-assembly, save, or game-mod access.
- The game mod and host own authoritative game state, legality, mutation, and host-thread authority.
- The gateway owns game-instance lifecycle, allocation, routing, leases, readiness, and fencing.
- The MCP server is a thin protocol adapter; the harness consumes it through an explicit port.
- The core remains free of transports, hosts, processes, and provider implementations.
- Provider credentials and network behavior enter only through an explicit provider port and policy.
- `sts2-protocol` is the accepted sixth implementation target for narrow, language-/transport-neutral
  shared contracts. The harness consumes its versioned schemas/packages but does not own a generic
  `common` crate, duplicate protocol authority, or protocol implementation internals.

Keep runtime communication and compile-time dependencies as separate graphs. A runtime call to a
gateway, MCP server, provider, or artifact store does not authorize a dependency on its implementation
crate. The harness must not bypass the MCP/gateway path to reach a game instance.

## Evidence and provenance

Use these evidence states precisely: `confirmed`, `source-derived`, `inferred`, `proposed`,
`unverified`, and `unsupported`. A schema parse, model response, reachable process, acknowledgement,
or recorded trajectory does not prove completed game action, semantic correctness, reproducibility, or
runtime compatibility.

Every reproducible record must bind the relevant independent versions, identifiers, configuration,
inputs, outputs, evaluator, and artifact digests. Keep `instance_id`, `session_id`, `run_id`,
`episode_id`, `trajectory_id`, `request_id`, `action_id`, `trace_id`, `model_execution_id`, and
`artifact_id` in their owning namespaces. Never collapse them into a generic correlation field.

Do not retain credentials, private prompts or model output, valued saves, proprietary host files,
personal paths, multiplayer identifiers, or unsanitized game text. Record origin, license, generator,
input identity, and hash for imported or generated fixtures. Copied reference implementations are
never an acceptable fixture or size exemption.

## Before editing

1. Read the applicable canonical documents and the ownership/dependency ADRs.
2. Inspect the target tree and any dirty state without resetting, cleaning, staging, or overwriting.
3. Identify the owned boundary and the observable contract affected.
4. Preserve unrelated files, including interop experiments and local work in shared checkouts.
5. Use `apply_patch` for file edits and keep changes within this repository.
6. Decide the validation and evidence needed before adding implementation or contract text.

Do not initialize Git, commit, push, merge, deploy, install, launch a game or provider, use
proprietary game files, or mutate a profile/save unless a later task explicitly authorizes that action.
Do not use broad staging or destructive cleanup.

## Implementation rules

When implementation is authorized, use the pinned Rust toolchain and project-owned requirements.
Keep modules cohesive, ports explicit, queues bounded, cancellation and shutdown owned, and errors
typed. Validate untrusted input at boundaries. Do not use `unwrap`, `expect`, `panic!`, `todo!`, or
`unimplemented!` in production paths. Unsafe code is forbidden except inside a separately approved
boundary with written invariants and focused tests.

Keep protocol/record types separate from coordinator policy, provider adapters, storage, transports,
and test support when their invariants differ. Do not create `common`, `utils`, or `manager` modules
as catch-alls. Do not add a transport to core merely because an adapter is convenient.

## Required validation and handoff

Run the local policy command for every change:

```bash
cargo run --locked --package repo-policy -- --strict
```

For Rust source changes, also run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

If a command is unavailable, requires a product/game workspace, or is intentionally skipped, report it
as unverified with the reason. The completion report must state what changed, affected contracts,
exact commands/results, remaining risks, and whether anything was committed, pushed, merged, released,
installed, deployed, or launched.
