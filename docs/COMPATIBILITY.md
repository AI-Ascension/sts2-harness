# Compatibility Policy and Matrix

The Runtime-v1 copied checksum inventory and golden messages were completed from protocol
`11e4252e39a77f0017b8e4f3720590e6162e8f53` during the 2026-09-05 review. Existing schema and
manifest bytes are unchanged. CI checks the copied POC, Runtime-v1, and Runtime-v2 inventories;
this confirms artifact integrity only, not host compatibility.

## Independent compatibility axes

“Compatible” is not one claim. The harness records these independently:

- harness API/CLI and record contract;
- `sts2-protocol` release, schema/profile, and conformance contract;
- trajectory, artifact, scoring, and training/dataset schema versions;
- provider profile and model execution contract;
- MCP server revision and gateway API/control-plane contract;
- game-mod/host versions observed through those boundaries;
- operating system, architecture, Rust/runtime/toolchain; and
- persisted configuration, artifact package, and migration format.

The harness does not inherit source or behavioral compatibility from a reference implementation.

## Current evidence baseline

This target contains a non-empty preparation package and one bounded authorized live-host integration trace.
No provider, model, replay service, score, dataset, or released package has been executed or
supported by the target. Compatibility claims remain limited to the exact runtime row recorded below.

| Subject | Current state | Evidence |
|---|---|---|
| Harness foundation | Pure ports, coordinator seams, and deterministic fakes | Source-derived; offline tests pass |
| MCP/gateway integration | Real component trace against synthetic downstream and exact host | Confirmed for the bounded runtime-v1 path; broader host compatibility unverified |
| Provider/model execution | Not executed | Unverified; no credentials or provider calls |
| Game state/action behavior | Not reachable by design | Unsupported in this repository boundary |
| Replay/artifact lineage | Offline record/replay and metadata seams | Source-derived; deterministic fakes only |
| Scoring | No scoring policy implementation | Proposed contract only |
| Runtime-v2 coordinator | Four-lane bounded pure scheduler with explicit lineage, fairness, overload, cancellation, and shutdown seams | Confirmed by offline component tests; live supervisor/profile/host isolation unverified |

## Compatibility classifications

Use `contract-compatible`, `additive-compatible`, `deprecated-compatible`, `safety-correction`, or
`breaking`. Every change identifies affected record fields, identifiers, versions, mappings, fixtures,
consumers, migration, and unverified evidence. Do not call a successful parse, acknowledgement,
action acceptance, or recorded trajectory runtime-compatible.

## Version and lineage rules

Keep harness, trajectory/schema, scoring, training/dataset, provider profile, MCP, gateway, game-mod,
protocol, host, and runtime versions independent. A run or artifact manifest binds each input that can affect its
observations, actions, model output, score, replay result, dataset bytes, or package bytes. Digests
bind exact inputs and outputs; wall-clock timestamps do not establish event order.

## Promotion evidence

Future support advances through deterministic offline tests, fake boundary/component tests, real
MCP/gateway integration, approved provider tests, disposable host smoke, focused runtime, and full
conformance. Each level requires exact versions, platform, configuration, artifact hashes, and date.
Missing credentials, services, game files, or disposable data remain visible as `unverified`.

## Runtime-v3, co-op, and patch evidence

The Runtime-v3 gameplay contract is source-derived from the neutral protocol profile and is mapped
through the MCP and gateway seams. The Exo adapter accepts only the sanitized fair-play projection
and current host action IDs. The co-op lane is additive and suspends mutation on peer disagreement,
missing peers, or disconnect. These are source/test claims; target-build and live provider behavior
remain `unverified`.

M10 records build, data, UI, action, and schema dimensions independently in
[`build-manifest.json`](evidence/runtime-v3-preparation/data/build-manifest.json). The manifest
is deliberately `quarantined` until exact package hashes, licensed-host traces, independent leak
checks, cleanup, replay, rollback, and all repository gates are available.

## Breaking changes

Breaking changes require an ADR, migration path, release note, updated fixtures/conformance, and
coordinated consumer review. Additive fields must define old-reader behavior. Unknown fields/enums,
null versus missing, ordering, numeric bounds, identifier namespaces, stale state, and partial effects
must be tested before an additive label is used.

## Runtime coordinator row

| Coordinator | Downstream lane | Current evidence | Result |
| --- | --- | --- | --- |
| `sts2-harness-runtime` | `runtime-v1-mcp` -> attached gateway -> STS2 v0.107.1 host | Authorized disposable-host trace | Bounded client-to-host probe confirmed; gameplay mutation and broader compatibility unverified; [evidence](evidence/runtime-v1-host-integration-20260902.md) |

The coordinator does not inherit compatibility from a successful trajectory. Promotion requires an
exact mod/host version, artifact digest, disposable profile, request sequence, fresh observation,
and successful cleanup. The runtime adapter binds `STS2_MCP_SESSION_ID` separately from
`STS2_SESSION_ID`, defaulting to `mcp-session-1` and requiring the two values to differ.
Distinct sessions require the corresponding session-binding updates in MCP #7 and gateway #6;
the response envelope remains bound to the gateway session.

## Runtime adapter safety correction

The gateway endpoint must now be a numeric loopback socket address (IPv4 or bracketed IPv6), not
a DNS name or remote plaintext address. One five-second exchange budget covers connect and every
partial HTTP read/write; ambiguous framing and raw response error payloads are rejected.
MCP exchanges likewise have a whole-call five-second default budget, bounded concurrent pipes, and
bounded direct-child cleanup. The configured MCP executable receives only explicit STS2 connection
configuration plus `PATH`, `SystemRoot`, `TEMP`, and `TMP`; stderr is suppressed. This is credential
minimization, not an operating-system sandbox or authority to execute an untrusted binary.
Descendant processes are not owned or forcibly killed, but cannot retain harness I/O workers.
Runtime-v2 tool responses must satisfy the copied contract and exact session/lease/request/operation
binding before evidence is used. The bounded `runtime-v3-gameplay` probe is not part of this branch.

Runtime-v1 consumes MCP's projected tool payload, not the full gateway envelope. The harness checks
outer JSON-RPC identity and strict projected kind/generation/observation/action/status/witness shape;
MCP owns gateway envelope and fence validation. A validated typed action rejection may carry
`isError: true` so the stale-generation oracle can inspect it. Arbitrary tool errors remain failures.

Foundation episode admission now releases mismatched router bindings and exposes cleanup failures.
This safety correction changes no serialized contract or frozen artifact bytes.

Allocation response rejection now attempts fenced cleanup without changing trace admission or any
frozen artifact bytes. A different lease returned for the requested instance, caller, and session
can be used only for cleanup after its identity and epoch are validated. Unattributable responses
retain the original configured fence. Cleanup failures remain explicit; live behavior is unverified.

The existing Runtime-v1 executable accepts only numeric loopback gateway socket addresses, not DNS
names or remote plaintext bearer endpoints. It bounds complete HTTP/MCP exchanges to five seconds,
validates outer RPC correlation and the existing projected tool contract, and minimizes child
environment/error output. MCP retains full downstream envelope/fence validation authority.
Frozen Runtime-v2 decoder fields remain required even when their permitted value is null.
