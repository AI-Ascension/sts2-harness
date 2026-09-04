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

The Runtime-v2 process records must also retain separate `instance_id`,
`gateway_session_id`, `mcp_session_id`, `lease_id`, `lease_epoch`, `run_id`, `episode_id`,
`trajectory_id`, and `artifact_id` values. They emit the actual MCP request-ID sequence and
downstream correlation IDs as bounded redacted fields. The record lineage defaults are suitable for
one deterministic probe and can be replaced with safe caller-supplied values through
`STS2_RUN_ID`, `STS2_EPISODE_ID`, `STS2_TRAJECTORY_ID`, and `STS2_ARTIFACT_ID`; duplicate lineage
values are rejected before the run.

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

## Runtime-v2 multi-instance coordinator seam

The pure `RuntimeV2Coordinator` tests cover the four-instance limit, cross-instance lineage
collisions, FIFO ordering within a lane, round-robin fairness across idle lanes, one serial active
slot per instance, global and per-instance queue overload, duplicate in-flight operation rejection,
queued cancellation, explicit active-operation shutdown reporting, retained operation identities,
and sanitized counters. The runtime adapter also propagates its separate MCP-session configuration.
These are
offline component tests. They do not establish a production supervisor, real process/port/profile
isolation, live gateway/MCP composition, host crash recovery, or gameplay settlement. The dated
result is recorded in [`runtime-v2-coordinator-20260902.md`](evidence/runtime-v2-coordinator-20260902.md).
The coordinator snapshot also records explicit unknown, rejection, and cancellation outcomes plus
optional dispatcher-supplied service-time samples, totals, and maxima globally and per instance;
it does not infer or retry an unknown operation. The regression for unknown completion verifies that
the same lane remains blocked until reconciliation while another instance continues to dispatch.

## Runtime process and control-plane failure probes

Workspace tests use synthetic children and loopback listeners for unread/full-duplex stdin,
oversized unterminated stdout, slow trickles, inherited descendant pipe handles, bounded close/drop,
wrong JSON-RPC IDs/versions, scoped Runtime-v2 response fences and malformed payload redaction.
HTTP regressions reject non-loopback/DNS targets, header injection, duplicate Content-Length,
Transfer-Encoding, oversized headers, and deadline extension by incremental response bytes.
These are local adapter checks, not game/provider execution or proof of a full connected runtime.
Trace stdout describes the checked action sequence; process exit success additionally requires
MCP shutdown and lease-release success. Cleanup errors remain reported even when the trace fails.

## Runtime adapter failure regressions

Allocation cleanup tests inject successful, mismatched, malformed, and unavailable gateway results.
They check one release attempt using only an attributable validated response fence or the original
configured fence, explicit cleanup failures, and no release during successful trace admission.
These deterministic offline tests do not contact a gateway or prove live lease release.

Synthetic subprocess tests exercise unread stdin, full duplex pipes, oversized/no-newline stdout,
slow trickles, inherited descendant handles, bounded close/drop, exact JSON-RPC identities and
async-caller spawn failure. Local listener tests enforce numeric loopback endpoints, request/header
validation, single Content-Length/no Transfer-Encoding, redacted errors and total deadlines.
Runtime-v1 reply tests cover projected generation/action/witness shape and typed stale rejection;
outer RPC identity tests stay separate from downstream envelope validation owned by MCP.
Runtime-v2 conformance parses all19goldens and rejects304 required-field omission variants. These
are safe local checks, not a new host/provider run or broader interoperability evidence.

## Runtime-v3, full-run, and co-op lanes

The copied [Runtime-v3 consumer contract](../protocol-artifact/runtime-v3-gameplay/README.md)
has an immutable upstream checksum inventory and exact schema-pin drift test. Offline JSON Schema
checks cover all four canonical goldens; actual observation and receipt parser tests consume the
two response goldens and reject selected malformed mutations. This bounded test lane does not
establish exhaustive conformance or cross-process/game/provider compatibility.

The `full_run` integration test routes setup, map, combat, reward, shop, event, rest, and selection
choices through a decision source, then records victory and defeat as separate terminal states. It
is a deterministic routing test, not a claim about target-game rules. Runtime-v3 tests additionally
cover sanitized Exo input, exact six-tool MCP exposure, stale/unknown results, bounded recovery,
and host-generated action binding. Focused tests also cover complete-run stage routing,
uncertain-dispatch reconciliation, and the bounded direct Exo process seam. The process tests use a
local shell only as a test fixture; production configuration invokes the operator-selected bridge
directly and never invokes a shell.

`deadline_includes_a_request_larger_than_an_unread_stdin_pipe` exercises a child
that never consumes a request larger than pipe capacity. The exchange must return
`Timeout` before that child would exit naturally. `drains_stdout_while_writing_a_large_request`
exercises a child that fills stdout before reading stdin, proving concurrent progress
in both directions. These are local subprocess checks, not provider evidence.

The Linux-only `exo_cleanup` test runs in a separate executable, starts finite descendants that
retain both pipes after their direct parent exits, and compares harness thread counts after four
timeouts, allowing at most 100 ms for Linux to retire joined task directories. It fails with eight
extra workers against the original blocking implementation. The async-pipe implementation must
have no extra harness workers within that bound, well before the two-second descendant exit.
Another test calls the synchronous transport from an existing async runtime. Neither test claims
that arbitrary bridge descendants are killed or that this process adapter provides OS isolation.

The runtime parser's settlement tests bind `from_generation` to the operation ledger's original
generation, including delayed waits after newer observations. They reject witnesses for a later,
unrelated transition and enforce canonical response-only null fields and status/error consistency.
These checks establish request-bound parsing, not the independence of an actual host witness.

Co-op tests cover two-peer identity, generation disagreement, disconnect, ally targeting, and
mutation suspension. They do not establish multiplayer host compatibility. The patch-diff utility
is source-only and compares bounded manifests; it cannot promote a build or replace package hashes.
Its workspace tests check bounded consumption even from an endless reader, exact-size admission,
invalid and ambiguous JSON rejection, structural quarantine extraction, and byte-preserving output.
`cargo test --locked --package sts2-patch-diff --test patch_manifest` validates the canonical M10
manifest using the full Draft 2020-12 schema and an independent quarantine assertion. Nested negative
cases exercise required fields, unknown fields, types, enums, lengths, array limits, and digest
patterns; this is structural evidence, not verification of the manifest's runtime claims.

The required M10 release evidence is recorded in
[`release-gate-preparation-20260904.md`](evidence/release-gate-preparation-20260904.md). Missing
Rust toolchains, licensed game assemblies, and live Exo/provider services make those gates
`unverified`, never passed by omission.
