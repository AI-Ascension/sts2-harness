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
trajectory replay, artifact metadata/lineage, one-time unbind/close cleanup, and the minimal POC.
The POC verifies the copied `poc-v1` artifact, runs a fixed-seed/fixed-clock fake slice through
`harness -> MCP -> gateway -> game-mod -> game-core`, and emits 15 canonical boundary events: five
for a state read, five for an accepted `use_budget` action, and five for a rejected zero-unit action.
It does not prove provider, live MCP, gateway, game, or artifact-store runtime behavior.

## Baseline commands

```bash
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

These commands are the local/CI entrypoint for the current workspace. They do not launch a game,
provider, gateway, MCP server, or external artifact store.

The POC parses the copied source/package schema, five goldens, invalid fixture, and conformance case,
checks their exact release checksums, and records the actual ordered fake-hop ledger. The report
records the exact trace and labels each claim as `confirmed` (deterministic fake only),
`source-derived`, `proposed`, or `unverified`: [`MINIMAL_POC_REPORT.md`](../MINIMAL_POC_REPORT.md).

## Coordinator and lifecycle tests

`foundation::router_cleanup` exercises rejected-binding cleanup and explicit unbind failure through
deterministic router doubles. This is component evidence only.

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

## Runtime coordinator checks

The runtime binary builds and is covered by the workspace Rust gates. A controlled component test
runs the real harness, MCP, and gateway binaries with a short-lived synthetic downstream. Its oracle
requires allocation identity, the runtime MCP catalog, generation N state, an accepted
`show_runtime_probe` response with a fresh visible witness, a stable stale-generation rejection,
post-action state at N+1, and lease release.

The synthetic result is component-network evidence only. A separate authorized host run exercised
the same coordinator path against the packaged mod and recorded the live host effect. The exact host
run and remaining unverified gates are recorded in
[`docs/evidence/runtime-v1-host-integration-20260902.md`](evidence/runtime-v1-host-integration-20260902.md).

## Runtime-v2 fake lane

The `sts2-harness-runtime-v2-fake` binary is an offline deterministic seam. It verifies the copied
Runtime-v2 artifact and emits one trajectory/artifact document for a single in-memory instance. Its
oracle requires a preallocated stable operation ID, admission-only acceptance, one post-write
disconnect recorded as unknown, fixed reconciliation with the same operation ID, a fresh settled
observation and witness at generation `N+1`, duplicate replay with one mutation, and stale-epoch
rejection before mutation. Provider/model execution and live host/game settlement are untouched and
remain `unverified`.

## Runtime adapter failure regressions

Synthetic subprocess tests exercise unread stdin, full duplex pipes, oversized/no-newline stdout,
slow trickles, inherited descendant handles, bounded close/drop, exact JSON-RPC identities and
async-caller spawn failure. Local listener tests enforce numeric loopback endpoints, request/header
validation, single Content-Length/no Transfer-Encoding, redacted errors and total deadlines.
Runtime-v1 reply tests cover projected generation/action/witness shape and typed stale rejection;
outer RPC identity tests stay separate from downstream envelope validation owned by MCP.
Runtime-v2 conformance parses all19goldens and rejects304 required-field omission variants. These
are safe local checks, not a new host/provider run or broader interoperability evidence.
