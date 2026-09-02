# Testing and Evidence

## Purpose

Tests must prove observable coordination, record, lifecycle, privacy, and artifact behavior rather
than merely execute lines. Most harness logic must run without a game, provider, MCP server, gateway,
socket, or wall clock. Controlled runtime lanes are separate and cannot be inferred from local tests.

## Test layers

| Layer | Purpose | Environment |
|---|---|---|
| Unit | ID/version validation, state transitions, ordering, errors, redaction | deterministic Rust |
| Record/protocol | exact encoding, optionality, unknown values, bounds, golden shapes | offline fixtures |
| Component | queues, lifecycle, ports, cancellation, fake provider/gateway/store | in-memory/bounded doubles |
| Integration | real MCP/gateway/provider/artifact boundaries | approved isolated environment |
| Conformance | project-owned observable requirements and mappings | CI plus controlled lanes |
| Host/runtime | game state/action effect and host compatibility | authorized disposable game environment |
| Release smoke | packaged bytes, manifests, checksums, startup and replay | clean supported environment |

The current target has policy-tool tests plus deterministic `crates/harness` tests. Those tests cover
route-to-episode correlation, retry reuse of model identity and idempotency, append idempotency,
trajectory replay, artifact metadata/lineage, and one-time unbind/close cleanup. They do not prove
provider, MCP, gateway, game, or artifact-store runtime behavior.

## Baseline commands

```bash
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

These commands are the local/CI entrypoint for the current workspace. They do not launch a game,
provider, gateway, MCP server, or external artifact store.

## Coordinator and lifecycle tests

Future tests must cover bounded capacity, FIFO or declared ordering, backpressure, overload,
admission, four-instance identity, allocation/lease handoff, stale fencing, cancellation before and
after acceptance, timeout/disconnect semantics, duplicate events, partial failure, restart/resume,
provider budgets, model-output bounds, and shutdown/join. Use deterministic barriers and injected
clocks; do not rely on arbitrary sleeps or retry-until-green behavior.

## Record, replay, and scoring tests

Test exact field names and encoding, missing/null/empty distinctions, unknown fields/enums, numeric
bounds, canonical ordering, namespace/lifetime of every identifier, version mismatch, redaction,
hash binding, and artifact lineage. Replay tests must distinguish input observations, requested
actions, accepted actions, completed effects, and unavailable evidence. Divergence and score tests
must identify evaluator version, inputs, policy, and partial-result behavior.

## Security and data tests

Test credential non-persistence, provider egress policy, prompt/model-output redaction, path and log
sanitization, cross-instance isolation, retention/deletion, artifact access, unbounded input
rejection, and fail-closed behavior when authorization or fixtures are missing. Never use valued
saves, public services, or another person's instance.

## Runtime evidence

Host or provider evidence requires exact versions, platform, configuration, disposable data, inputs,
outputs, cleanup, artifact hashes, and evidence level. A build, handshake, model response, reachable
process, acknowledgement, or trajectory is not proof of semantic correctness or completed game action.
Unavailable runtime remains `unverified` with an executable safe probe.
