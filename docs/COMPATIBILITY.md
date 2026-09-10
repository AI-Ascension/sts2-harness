# Compatibility Policy and Matrix

## Historical recovery consumer candidate

The additive `watchdog-recovery-v1` consumer accepts the schema-permitted standard/URL-safe
base64 alphabets, with or without complete padding, while rejecting invalid tail bits, malformed
padding, whitespace and oversized input. Canonical RCJ-1 bytes, frozen Runtime-v3 schema identity
and the retained payload digest still have to match exactly. No frozen artifact bytes change.

Reconcile responses may retain `SETTLED` or `REJECTED`, or record `RECONCILED`; the operation,
original context, result, ticket and witness must agree before closing durable uncertainty.
`NOT_FOUND` and unresolved states do not prove non-execution. See
[ADR 0011](decisions/0011-historical-recovery-evidence.md).
Per-operation original-context persistence and fresh allocation-authority handoff remain a separate,
unfinished integration dependency; environment-supplied context is not cross-boot recovery proof.

With `STS2_LIVE_EPISODE=true`, idle observation failures now log the harness-owned error code,
and MCP RPC failures log only the numeric RPC code. Remote error messages and data remain
suppressed. These diagnostics do not retry actions or change failure and cleanup behavior.

## Bounded model plans

The unreleased Rust `Decision` enum gains `Plan`; exhaustive Rust consumers must add a match
arm. The Exo response parser accepts `decision: "plan"`, `action_ids` (one to eight distinct
current IDs), and `rationale`. Older parsers reject this response, so the Astra bridge and
harness must migrate together. Single-action responses remain accepted. Runtime-v3 host
schemas and artifact bytes are unchanged. See [ADR 0008](decisions/0008-bounded-model-action-plans.md).

`DecisionSource` gains default settlement and originating-model-identity callbacks. The runner
and combat demo forward verified settlement; recording wrappers preserve the origin for cached
steps. Live action records retain `model_decision` for existing replay readers and add
`reused_model_execution`; multiple executed steps may share one `model_execution_id`.
Count distinct execution IDs for provider calls, and verified operation IDs for settled actions.
Unknown extra event fields are ignored by the existing combat replay reader.
Deterministic tests cover combat/shop rebinding and invalidation; live speed and full-run
compatibility remain unverified until separately recorded.

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

This target includes bounded live-host integration and an opt-in runtime-v3 combat demo.
On 2026-09-05 the Rust Ollama bridge completed a real combat through MCP/gateway with
18 settled actions, followed by fresh-process action replay. This does not establish a
deployed replay service, full campaign, experimental score/dataset result, or released package.
See `experiments/live-combat/README.md` for the exact scope.

| Subject | Current state | Evidence |
|---|---|---|
| Harness foundation | Pure ports, coordinator seams, and deterministic fakes | Source-derived; offline tests pass |
| MCP/gateway integration | Real component trace against synthetic downstream and exact host | Confirmed for the bounded runtime-v1 path; broader host compatibility unverified |
| Provider/model execution | Ollama gemma4:31b-cloud via structured provider port | Confirmed for the isolated single-combat demo |
| Direct game access | Outside the harness boundary; requests use MCP/gateway | No direct host authority; bounded indirect runtime-v1 probe only |
| Replay/artifact lineage | Offline seams and fresh-process combat action replay | Confirmed visible-state comparison for the demo; broader replay unverified |
| Runtime-v2 coordinator | Four-lane bounded pure scheduler with explicit lineage, fairness, overload, cancellation, and shutdown seams | Confirmed by offline component tests; live supervisor/profile/host isolation unverified |
| Runtime map context | Opt-in host-authored map projection carried to the Exo provider | Source/component evidence only; target-build map production, provider behavior, and live compatibility unverified |
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

### Runtime-v4 expert source/component row

At current harness main
[`b8c50c87db0275f0e08d69892f1ebce275f4acb6`](https://github.com/AI-Ascension/sts2-harness/commit/b8c50c87db0275f0e08d69892f1ebce275f4acb6),
the coordinator consumes the copied `runtime-v4-expert` and `runtime-v4-expert-action` artifacts,
maps the expert MCP catalog, validates the fair-play observation and legal-action bindings, and
exercises bounded executable composition and recovery checks. Their schema digests are
`0ee034d5da83f34e9fa0ba23038738d56ef8cfccb1c6e752af3ab63d212c8e42` and
`393318bda8c3522c0ecbacc78b95471a9f4dc3f825169d2048f4c74a7b7f2929`; the copied protocol source
is aligned with protocol main `f2dac90529f584a6511c1760adce9da28f7f910a`. The separate
`runtime-v4-expert-rest-action` artifact remains a candidate at digest
`bb3555fae28eb1f79d08a15e9884696a579e4c20836f5016509f17e0f4c36fbd`. This is source/component
and bounded synthetic composition evidence; native host legality, settled effects, provider-run
compatibility, deployment, release, and live end-to-end behavior remain unverified.

### Runtime map context

Map context is an additive, opt-in provider-request capability. `STS2_ENABLE_MAP_CONTEXT` defaults to
`false` and accepts only the exact values `true` or `false`. The runtime coordinator accepts the
map flag with both the `runtime-v3-gameplay` and `runtime-v4-expert` profiles; this source-level
profile composition is not evidence that a native target supports the combination. Enabling it
requires an Exo bridge that accepts the
`sts2.exo-decision-map-v1` request and its `map_context` field. A bridge that only accepts
`sts2.exo-decision-v1` is incompatible with map-stage requests; the runner fails closed rather than
falling back to an ordinary request or choosing a heuristic action.

At a Map-stage observation, after the current observation and host legal-action catalog have been
validated, the runtime starts a short-lived `runtime-map-v1` MCP profile. That profile must expose
the ordered seven-tool catalog ending in `sts2.map_snapshot`. The returned envelope is checked for
the `runtime-map-v1` profile, schema digest
`ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b`, exact identity and generation,
and the hand-authored `sts2-protocol/runtime-map-v1` provenance. Only a snapshot marked
`available`, `complete`, and `current` is accepted. Its graph is bounded at 256 KiB, 256 nodes,
1,024 edges, and 256 action bindings; the graph must be acyclic. Every binding must correspond to
the current host-generated `select_map_node` action ID set. The snapshot is canonicalized and sent
with its digest, state ID, generation, profile, and schema digest in `map_context`.

The 256 KiB snapshot bound applies only to the serialized snapshot body; it does not guarantee that
the enclosing raw native, gateway, or projected-MCP whole envelope fits. Those whole-envelope paths
remain bounded at 256 KiB. Ordinary MCP stdout remains bounded at 256 KiB; only the map-profile
framed stdout allowance is 512 KiB so escaped snapshot JSON and its JSON-RPC/content wrapper can be
read. These are separate bounds, so the lower-level envelope checks remain authoritative and a
snapshot near its own limit may still be rejected after wrapping. The complete Exo request bound is
393,443 bytes, derived from the ordinary
131,072-byte request bound, the 256 KiB snapshot bound, and the fixed map wrapper. With map context
enabled, `STS2_EXO_MAX_REQUEST_BYTES` defaults to and must equal `393443`; the provider response
remains bounded at 8 KiB. Other episode stages continue to use the ordinary `sts2.exo-decision-v1`
request. Map visibility does not change action authority: the host still supplies and validates the
typed legal-action payloads.

This capability is confirmed only by source and deterministic/component tests in this target. Map
production by a target build, live MCP/gateway wiring, provider interpretation, gameplay settlement,
and compatibility beyond the recorded fixtures remain `unverified`. Operator setup and the data
boundary are documented in [`experiments/exo-agent/README.md`](../experiments/exo-agent/README.md).

The co-op library gate suspends local admission when a registered peer is reported disconnected or
disagrees with its fixed generation snapshot. It cannot detect missing members of an expected roster:
no such roster is configured, and local-only registration can pass. It also has no API to advance
the coordinator generation. Snapshot checks do not establish continuous two-to-four-peer operation,
authoritative membership, or multiplayer host compatibility; these require a defined contract and
runtime integration before any stronger guarantee.

This target-local helper has no co-op wire schema, profile, digest, MCP tool or runtime transport.
The admitted protocol profile is `coop-synchronization-v1`, produced by gateway serialization and
read by MCP as coordinator-reported metadata; it carries no action, vote, shared-effect, or host-game
authority. This harness helper does not produce or consume that wire profile. Exporting
`CoopCoordinator` therefore remains a local source/component check, and its source does not establish
native peer admission, actions, votes, shared effects, or disconnect/rejoin recovery. Co-op digests in
dated preparation records describe the preserved unadmitted gameplay proposal, not this admitted
read-only synchronization profile.

M10 records build, data, UI, action, and schema dimensions independently in
[`build-manifest.json`](evidence/runtime-v3-preparation/data/build-manifest.json). The manifest
is deliberately `quarantined` until exact package hashes, licensed-host traces, independent leak
checks, cleanup, replay, rollback, and all repository gates are available.

## Breaking changes

### Runtime-v3 canonical artifact provenance

The [Runtime-v3 bundle](../protocol-artifact/runtime-v3-gameplay/README.md) is copied byte-for-byte
from the current `AI-Ascension/sts2-protocol` main
`f2dac90529f584a6511c1760adce9da28f7f910a` (MIT). The canonical
`SHA256SUMS`, README, manifest, schema and seven goldens retain upstream bytes; the
[source schema](../schemas/runtime-v3-gameplay.schema.json) and
[conformance case](../conformance/cases/runtime-v3-gameplay.json) preserve the inventory's relative
paths. Earlier relocated `UPSTREAM_SHA256SUMS` and `conformance.json` copies are removed.

Schema SHA-256 is `8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63`;
the authoritative inventory SHA-256 is
`ddc7c0a3697bcb474de8e7967041302dab072e11bc9990ffd5a508eb391cc1db`.
This coordinated revision admits proceed, confirm-selection and cancel-selection in the typed
policy and Exo action parser. Earlier digests fail closed; all consumers must migrate together.
Regenerate only by copying the complete bundle and its source/conformance mirrors from a reviewed
protocol revision, then reviewing both pins and provenance. Goldens are upstream hand-authored
synthetic observations/actions; no host files, credentials, provider output or private data occur.
Tests verify every checksum entry and validate seven goldens against the schema, with response
goldens also traversing the actual observation/receipt parsers. Request goldens receive schema
validation; continuation payloads additionally traverse the policy observation parser and reject
extra arguments. This is bounded source/component evidence; broader transport, Exo, host settlement
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

## Exo projection safety correction

The default Exo decision request preserves the host `visible_seed`, following the owner's
requirement for repeatable invocation and replay. `ExoConfig::forward_visible_seed` defaults
to `true`; `with_visible_seed_forwarding(false)` or `STS2_EXO_FORWARD_VISIBLE_SEED=false`
selects seed-blind behavior (exact `true`/`false`, anything else rejected).
`ExoConfig` gained that public field; callers using struct-update syntax are unaffected. The
`sts2.exo-decision-v1` request root now treats `visible_seed` as optional while the other five root
fields stay required. This changes what an operator-owned bridge receives by default, not any
frozen artifact, schema, golden, or checksum; the host-facing `runtime-v3-gameplay` contract still
requires the field from the host. Behaviour is `confirmed` by the `provider_redaction` tests.

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
