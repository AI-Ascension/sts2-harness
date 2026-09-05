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
No live provider/model execution, deployed replay service, experimental score/dataset result, or
released package is established by this evidence. Offline replay/evaluation library tests are a
separate layer. Runtime compatibility claims remain limited to the exact row recorded below.

| Subject | Current state | Evidence |
|---|---|---|
| Harness foundation | Pure ports, coordinator seams, and deterministic fakes | Source-derived; offline tests pass |
| MCP/gateway integration | Real component trace against synthetic downstream and exact host | Confirmed for the bounded runtime-v1 path; broader host compatibility unverified |
| Provider/model execution | Not executed | Unverified; no credentials or provider calls |
| Direct game access | Outside the harness boundary; requests use MCP/gateway | No direct host authority; bounded indirect runtime-v1 probe only |
| Replay/artifact lineage | Offline record/replay and metadata seams | Source-derived; deterministic fakes only |
| Runtime-v2 coordinator | Four-lane bounded pure scheduler with explicit lineage, fairness, overload, cancellation, and shutdown seams | Confirmed by offline component tests; live supervisor/profile/host isolation unverified |
| Evaluation | Library aggregation over supplied samples; not wired into the Runtime-v3 runner | Synthetic tests, not game parity or experimental performance evidence |

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

Decision replay now uses the `decision-replay-v2` non-cryptographic comparison fingerprint,
binding every current record field and correlation identity with explicit optional markers and
length-delimited text/payload bytes. This safety correction intentionally changes earlier
unreleased fingerprint values; regenerate comparisons from retained input records. It is not
an integrity digest, evidence validation, or a substitute for independent version/artifact lineage.
`DecisionPayload` and `DecisionMemory` require caller classification and redaction: their
bounded JSON and forbidden-key checks do not detect private content or authorize storage/export.

## Promotion evidence

Future support advances through deterministic offline tests, fake boundary/component tests, real
MCP/gateway integration, approved provider tests, disposable host smoke, focused runtime, and full
conformance. Each level requires exact versions, platform, configuration, artifact hashes, and date.
Missing credentials, services, game files, or disposable data remain visible as `unverified`.

## Runtime-v3, co-op, and patch evidence

The Runtime-v3 gameplay contract is source-derived from the neutral protocol profile and is mapped
through the MCP and gateway seams. The Exo adapter accepts only the sanitized fair-play projection
and current host action IDs. The executable assembles one configured instance's episode/provider
path, not the separate record, memory, evaluation, replay, artifact-publication or co-op library
seams. Target-build and live provider behavior remain `unverified`.

The co-op library gate suspends local admission when a registered peer is reported disconnected or
disagrees with its fixed generation snapshot. It cannot detect missing members of an expected roster:
no such roster is configured, and local-only registration can pass. It also has no API to advance
the coordinator generation. Snapshot checks do not establish continuous two-to-four-peer operation,
authoritative membership, or multiplayer host compatibility; these require a defined contract and
runtime integration before any stronger guarantee.

This target-local helper has no co-op wire schema, profile, digest, MCP tool or runtime transport.
The protocol co-op contract is a blocked proposal outside the admitted Runtime-v3 gameplay bundle;
exporting `CoopCoordinator` does not advertise protocol support. Co-op digests in dated preparation
records describe that historical proposal, not the currently admitted consumer artifact inventory.

M10 records build, data, UI, action, and schema dimensions independently in
[`build-manifest.json`](evidence/runtime-v3-preparation/data/build-manifest.json). The manifest
is deliberately `quarantined` until exact package hashes, licensed-host traces, independent leak
checks, cleanup, replay, rollback, and all repository gates are available.

## Breaking changes

### Runtime-v3 canonical artifact provenance

The [Runtime-v3 bundle](../protocol-artifact/runtime-v3-gameplay/README.md) is copied byte-for-byte
from `AI-Ascension/sts2-protocol` candidate `be0f3f230911f119dbe8e19c71e8249b22f53e59` (MIT).
This candidate must be checked against merged protocol main before consumer merge. The canonical
`SHA256SUMS`, README, manifest, schema and four goldens retain upstream bytes; the
[source schema](../schemas/runtime-v3-gameplay.schema.json) and
[conformance case](../conformance/cases/runtime-v3-gameplay.json) preserve the inventory's relative
paths. Earlier relocated `UPSTREAM_SHA256SUMS` and `conformance.json` copies are removed.

Schema SHA-256 remains `b37c80f583aeaf4f81ede2083bcfb4129196baf5eb092470e8738173c4b7226c`;
the authoritative inventory SHA-256 remains
`ec17dc526545c356462773f9e634ea7b25546c877c601cc1640eae3d7341cb81`.
Regenerate only by copying the complete bundle and its source/conformance mirrors from a reviewed
protocol revision, then reviewing both pins and provenance. Goldens are upstream hand-authored
synthetic observations/actions; no host files, credentials, provider output or private data occur.
Tests verify every checksum entry and validate four goldens against the schema, with response
goldens also traversing the actual observation/receipt parsers. Request goldens receive schema
validation only. This is bounded source/component evidence; broader transport, Exo, host settlement
and live compatibility remain unverified. Frozen Runtime-v1/v2 bytes are unchanged.

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
The persistent MCP child uses cancellable asynchronous pipes and joined supervisors; direct-child
shutdown/reaping is bounded and errors remain visible. Descendants are not forcibly killed, but
inherited pipe handles cannot strand harness I/O workers. Only explicit STS2 connection variables
plus PATH/SystemRoot/TEMP/TMP are inherited, and stderr is suppressed. This is credential
minimization, not an OS sandbox. MCP and gateway sessions are separate namespaces; the configured
MCP child receives both identities explicitly, and its adapter must bind them without equating them.
The six-tool Runtime-v3 catalog is independent of the retained Runtime-v2 four-lane scheduler.
Configure the same explicit `STS2_MCP_SESSION_ID` in the independently launched gateway and harness.
Harness, gateway, and MCP default to `mcp-session-1`; custom session names require coordinated
configuration. The gateway session independently defaults to `session-1`.

Dispatch preserves the complete host legal-action reference (`action_id` plus typed `action` payload)
across the MCP boundary. A bare payload is not a legal-action reference. Canonical schema regressions
cover end-turn and card payloads, including explicit nullable card targets.
