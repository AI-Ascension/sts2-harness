# Compatibility Policy and Matrix

The additive Exo lookup source profile uses `sts2.exo-lookup-wire-v1` and distinct
`sts2.exo-lookup-config-v1`; see [ADR 0022](decisions/0022-exo-lookup-duplex-bridge.md).
Its process adapter requires an explicit authoritative owner binding. Automatic native
episode integration is blocked on an accepted binding discovery/observation contract.

The additive game-information consumer pins the exact v1 schema independently of the Rust
protocol dependency. Its synthetic component matrix and pending mixed-catalog/provider gates
are recorded in [ADR 0039](decisions/0039-game-information-consumer.md). Public static/player
live lookups, bounded retained pages and encrypted restart replay are component-tested;
native/provider execution and the old native Exo adapter's tool translation remain unverified.

The additive lookup-binding consumer pins
`game-information-lookup-binding-v1` at schema digest
`f10f9af01d6be1de104069ba842e7971971e88f27553e782e81174ee7aa1cd58`.
It uses the gateway-owned discovery/observe route only when explicitly enabled, rejects ownership
overrides, and fails closed on unavailable native adapters or exhausted re-observation. This does
not add a complete content-manifest transport; its missing package/version/order/inventory and
override fields require a separately versioned contract. See
[ADR 0043](decisions/0043-game-information-lookup-binding-route.md).

The additive live-observation bootstrap consumer pins `game-information-live-observation-bootstrap-v1` at
digest `6041a282ffda8757af4e3eb6ab551e082f136fe53138ab8ac17db9fab52765c2` and consumes the shared conformance
case (sha256 `b16ed9ebd584131d72bca3fb3978fd5d32e44a5e9d695e98d10f58b99fb01469`) with its invalid fixtures
`cross-run` `50729b0b…`, `duplicate-visible-entity` `b7ab7a81…`, `foreign-instance` `87ec2981…`, `foreign-manifest`
`e9b5bb77…`, `missing-native-ref` `549d5bc1…`, `selector-instance-mismatch` `a3e585eb…`, `stale-generation`
`719210fb…` (full digests in `SHA256SUMS`); a `not_observable` error maps to a typed missing-capability result.

## Bounded parallel analysis route

`execute_plan_bounded` is `additive-compatible`: new `ParallelAnalysisExecutor`, `ParallelCap`, `BranchOutcome`,
`JoinedResult` and `DynamicPlanError::{UnsettledInput, BranchUnknown, BranchLost}` beside the unchanged serial
`execute_plan`; it reads the existing `max_parallel_analyses` (`1..=4`) and adds no `NodeKind`, schema, route,
record or Studio pin. Cap=1 reproduces the serial values for executors reading declared inputs only; a failed or
unknown branch never feeds a decision; budget, cancel/restart and browser branch states stay open. See [ADR 0049](decisions/0049-bounded-parallel-analysis-join.md).

## Benchmark manifest foundation

`benchmark_manifest` adds a private `ascension.benchmark-manifest.v1` owner format and
an effect-free Rust API. This is `additive-compatible`: it changes no old record,
seed normalization, native wire artifact, database or runtime admission. Strict readers
reject unknown versions/fields, duplicate members and incomplete required inputs.
There are no compatibility relaxations; equal declarations do not establish runtime support.
Receipt binding also requires the declared seeded-start protocol version and schema digest
to match the receipt. `receipt_protocol_mismatch` corrects missing validation in the unreleased
candidate; it changes no frozen wire schema or existing runtime receipt interpretation.
See [ADR 0021](decisions/0021-benchmark-manifest-foundation.md) for exact identity and
receipt evidence limits. The supported input context remains standard Ironclad,
ascension 0..20, with a fresh baseline; other modes/characters remain unsupported here.
Seed generation/durability, complete native readback/RNG, cold-launch integration,
profile provisioning and provider execution remain unverified and outside this delivery.

## Context-owner management surface

The rows for recorded context bindings ([ADR 0040](decisions/0040-recorded-context-binding-history.md)),
the composed effective-limits projection ([ADR 0030](decisions/0030-context-owner-effective-limits-composition.md)),
selected limit enforcement ([ADR 0028](decisions/0028-selected-context-control-limit-enforcement.md),
[ADR 0041](decisions/0041-selected-limit-enforcement-wiring.md)), the current association
([ADR 0025](decisions/0025-context-owner-current-association.md)), receipt recovery
([ADR 0024](decisions/0024-context-control-receipt-recovery.md)), control commands
([ADR 0048](decisions/0048-context-owner-control-commands.md)) and the recorded-binding projection
([ADR 0023](decisions/0023-recorded-context-binding-http-projection.md)) are kept together in
[`COMPATIBILITY_CONTEXT_OWNER.md`](COMPATIBILITY_CONTEXT_OWNER.md).

## Served provider-session effective-limits record

[ADR 0052](decisions/0052-provider-session-effective-limits-route.md) adds
`GET /v1/workflow-runs/{run_id}/provider-session-effective-limits` (`workflow:read`), returning the
producer's `ascension.harness.effective-limits.v1` record built by
`NativeCapabilities::effective_limit_record` from the descriptor the served process admits provider
sessions against, and `GET /v1/workflow-runs/{run_id}/context-memory-effective-limits`, which the
served workflow composition refuses with the typed `context_memory_record_unavailable` because it
holds no memory corpus. This is `additive-compatible`: the record schema, its producers, the
fixtures under `fixtures/effective-limits/` and every existing route are unchanged, and no bound,
digest or default changes. The record is metadata only (classification rows and the descriptor
digest; no session content, credential or policy bytes) and is fenced to the run's current
context-owner association, whose boundary must name the served descriptor's adapter and model
revision (`provider_session_capabilities_mismatch` otherwise). A composition without a served
descriptor answers `provider_session_capabilities_unavailable`; it never falls back to a fixture.
Evidence is synthetic and in-process; the sidecar population on the Console side is a separate
consumer change.

## Saved provider-session policy migration

[ADR 0027](decisions/0027-provider-session-policy-migration.md) adds
`SessionPolicyMigrationProposal`, which records a saved policy that is portable-schema valid but
above the selected profile's executable ceiling. It retains the exact saved bytes and the violated
limits, requires explicit approval, and adopts only a caller-supplied target that is itself within the
executable ceilings (`effective_limit_exceeded` otherwise). This is `additive-compatible` and
library-only: no policy field, schema, digest, range or bound changes, and nothing is clamped,
persisted or activated.

## Saved provider-session policy admission

[ADR 0026](decisions/0026-provider-session-saved-policy-admission.md) adds
`ProviderSessionPolicy::admit_for_profile`, which classifies a saved policy against the selected
adapter profile: portable-contract failures (`provider_session_policy_schema_invalid`) stay distinct
from schema-valid-but-unexecutable values (`effective_limit_exceeded`, `disabled`,
`field_not_advertised`, ...). This is `additive-compatible`: no policy field, schema, digest, range or
resource bound changes, and `validate`/`validate_schema` are unchanged. The portable schema ceilings
stay intentionally broader than the executable ceilings, and a refused policy is never clamped.

## Effective-limit consumer alignment

The matrix records Console `df36452adcfa1b1c3a7f968be243cd25a02433c3` and Studio
`31c5e5f407ab17fb0363374dd1d926c25e6350ea` as v3 aligned consumers for context-memory and
provider-session. Console copies all four unchanged producer schemas; Studio uses a versioned
adapter. Each revision is enforced by a real harness CI checkout and candidate conformance lane.
The existing Studio workflow-owner browser regression is repinned deliberately and retained.

This completes a coordinated consumer schema migration, without changing producer schema bytes,
resource ceilings or public matrix shape. Console retains explicit v1 rollback; pending consumers
remain unavailable. The new requirement that aligned entries have a matching repository-owned
CI pin is a fail-closed correction to the unreleased pin validator. See the
[candidate conformance contract](../tools/consumer-conformance/README.md).
The checkout validator parses the existing static YAML lanes with exact `yaml-rust2` 0.13.0
(default features disabled); conditional/ambiguous workflow shapes cannot establish alignment.
This adds a package dependency and root lockfile entries, without changing workflow bytes or
public schemas. Static checkout validation does not replace terminal hosted conformance results.

Evidence remains synthetic for effective-limit publication/admission. Context owner transport,
native/provider integration and the broader harness #95 / Studio #119 feature gates remain
unverified; this matrix does not imply their completion or activate saved policies.

## Historical recovery consumer candidate

The additive `watchdog-recovery-v1` consumer accepts the schema-permitted standard/URL-safe
base64 alphabets, with or without complete padding, while rejecting invalid tail bits, malformed
padding, whitespace and oversized input. Canonical RCJ-1 bytes, frozen Runtime-v3 schema identity
and the retained payload digest still have to match exactly. No frozen artifact bytes change.

Reconcile responses may retain `SETTLED` or `REJECTED`, or record `RECONCILED`; the operation,
original context, result, ticket and witness must agree before closing durable uncertainty.
`NOT_FOUND` and unresolved states do not prove non-execution. See
[ADR 0011](decisions/0011-historical-recovery-evidence.md).
Runtime-v3 schema migration 6 now persists the canonical per-operation original allocation context
before dispatch, while the fresh allocation supplies only the current fence for a recovery lookup.
Legacy operation rows without that field remain explicitly non-recoverable; synthetic cross-boot
consumer tests still do not establish a live gateway/host reboot result.

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

### Pinned Exo executor bridge

The harness-owned `sts2-exo-bridge-v1` contract is documented in
[ADR 0017](decisions/0017-exo-executor-bridge-contract.md) and frozen in
[`protocol-artifact/exo-bridge-v1`](../protocol-artifact/exo-bridge-v1/README.md). Its source
revision, package/executable, dedicated TypeScript extension, bounded bridge, model binding,
prompt/tool/configuration digests, contract version, and native instance identity are separate
compatibility axes. The selected executor path is the extension's
`defineHarness.runTurn` → `runResponsesHarnessTurn` → `ResponsesRuntime.complete` chain; HTTP
substrate requests, `/health`, and the human-facing CLI are not fallback executors.

The closed capability/preflight descriptor and request/turn envelope drive the production
admission gate recorded in [ADR 0031](decisions/0031-runtime-exo-admission-gate.md). The reviewed
`STS2_EXO_ADMISSION=envelope` mode refuses the run while a required capability, digest, revision,
route or schema is not admitted, and it refuses before the durable store, the gateway, the MCP
session, the provider or any game effect exists; `STS2_EXO_ADMISSION=legacy` is an explicit
operator acknowledgement of an un-admitted raw-wire bridge. That gate cross-checks the identity
**inspected** from the launch's own artifacts against the operator pin
([ADR 0032](decisions/0032-inspected-admission-identity.md)), so a swapped package, extension or
bridge artifact fails closed and a pinned axis the inspection did not bind refuses as
`UnboundIdentity` instead of being admitted on the declaration alone. Standard/fresh/Linux x86_64 and
strict terminal decision parsing are source-derived;
map/expert, continuity, cancellation/recovery, event/usage, replay, native package/model
identity, live Exo connectivity, and STS2 gameplay remain `unverified` until the real pinned
executor spike records them. The required `runtime`, `provider`, and `endpoint` identity axes are
classified as a `breaking` required-configuration correction; readers that cannot validate them
fail closed. The tightened expert digest and non-empty legal-action schema, plus parser-only
checks for duplicate action IDs, UTF-8 byte bounds, and `hp <= max_hp`, are a
`safety-correction`. Schema-valid/parser-rejected semantic cases are executable conformance
vectors. Migrations must stage additive records, run schema and trusted preflight checks, retain a
backup until handoff, and restore that backup on failure without reinterpreting new records as
legacy `provider_revision`-only identities.

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
| Seeded-run transport | Opt-in bounded plan, context digest, durable reservation, and same-operation MCP recovery | Source/component and artifact checks confirmed; native seed settlement, profile/save isolation, gameplay, deployment, and release unverified |
| Runtime map context | Opt-in host-authored map projection carried to the Exo provider | Source/component evidence only; target-build map production, provider behavior, and live compatibility unverified |
| `coop-native-v1` harness consumer | Strict parser, bounded cohort coordinator, and canonical-peer attribution for the accepted native co-op artifact | Source/component tests cover all seventeen goldens, peer-token uniqueness, generation fences, effects/receipts, same-operation recovery, cohort route binding, and canonical local-peer/actor plus route-fence rejection; MCP/HTTP transport, native settlement, checksum convergence, deployment, and release unverified |
| Evaluation | Library aggregation over supplied samples; not wired into the Runtime-v3 runner | Synthetic tests, not game parity or experimental performance evidence |

## Compatibility classifications

Use `contract-compatible`, `additive-compatible`, `deprecated-compatible`, `safety-correction`, or
`breaking`. Every change identifies affected record fields, identifiers, versions, mappings, fixtures,
consumers, migration, and unverified evidence. Do not call a successful parse, acknowledgement,
action acceptance, or recorded trajectory runtime-compatible.

### Runtime-v3 telemetry identity privacy correction

Runtime-v3 telemetry is a `safety-correction`: all serialized harness lineage attributes
(`sts2.run_id`, `sts2.episode_id`, `sts2.trajectory_id`, `sts2.trace_id`,
`sts2.instance_id`, and `sts2.session_id`) use deterministic domain-separated SHA-256 digests,
as do operation and action identifiers. `sts2.id_encoding=digest` applies to every emitted span.
The raw trace lineage remains private process state solely for stable OTLP trace/span derivation;
it is not serialized as an attribute.

Operators must query the backend using the same domain-specific digest of an access-controlled raw
run identifier. Raw-to-digest correspondence remains in local run evidence only and must not be
copied into OTLP, exporter logs, terminal evidence, or a public artifact. Existing backend queries
that predicate on raw identities are incompatible and must be updated before use. This source and
component correction does not establish collector, backend, deployment, or live-runtime evidence.

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
[`3926e5a30ab569612e67d2dfdc6542f1391e95d7`](https://github.com/AI-Ascension/sts2-harness/commit/3926e5a30ab569612e67d2dfdc6542f1391e95d7),
the coordinator consumes the copied `runtime-v4-expert` and `runtime-v4-expert-action` artifacts,
maps the expert MCP catalog, validates the fair-play observation and legal-action bindings, and
exercises bounded executable composition and recovery checks. Their schema digests are
`0ee034d5da83f34e9fa0ba23038738d56ef8cfccb1c6e752af3ab63d212c8e42` and
`393318bda8c3522c0ecbacc78b95471a9f4dc3f825169d2048f4c74a7b7f2929`; the copied protocol source
was admitted from protocol main `f2dac90529f584a6511c1760adce9da28f7f910a`; current protocol main
is `d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404`. The separate
`runtime-v4-expert-rest-action` artifact remains a candidate at digest
`bb3555fae28eb1f79d08a15e9884696a579e4c20836f5016509f17e0f4c36fbd`. This is source/component
and bounded synthetic composition evidence; native host legality, settled effects, provider-run
compatibility, deployment, release, and live end-to-end behavior remain unverified.

### Seeded-run transport source/component row

At current harness main
[`3926e5a30ab569612e67d2dfdc6542f1391e95d7`](https://github.com/AI-Ascension/sts2-harness/commit/3926e5a30ab569612e67d2dfdc6542f1391e95d7),
the opt-in transport validates a contiguous seed plan and a concrete standard Ironclad context,
establishes a generation fence with one read-only observation, and creates a durable reservation
before the single `start_seeded_run` mutation. The `seeded-run-v1` MCP profile is used for start and
bodyless reconciliation; `unknown` or disconnected starts retain the original operation ID and
never issue a new seed mutation. Settlement requires canonical seed readback, a fresh observation,
and the `run_started` witness. The copied protocol artifact is
`sts2-protocol/seeded-run-v1` at schema digest
`5c659f344be78f84e8d783986925d462714f933cac95d18943358992f7d3e2b8`, aligned with protocol main
`d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404`. These checks confirm the source/component boundary and
artifact integrity; native host settlement, profile/save isolation, gameplay, provider execution,
deployment, and release compatibility remain unverified.

### Native co-op source/component row

The harness consumes `sts2-protocol/coop-native-v1` as a component artifact with schema digest
`2f3bc99e53080fa11b39592b64fb0ab964a16f568719a2622d0b2caf766ab629`. The parser preserves the
producer's closed envelope and seventeen goldens, accepts bodyful same-operation recovery requests,
uses peer-token identity for roster uniqueness, and enforces the producer/gateway generation rules:
settled effects advance `before < after == observation.host_generation`, while non-settled effects
remain at the observation generation; recovery outcomes retain their status-specific receipt
relations. The coordinator records unknown operations and reconciles them with the original
operation identity. This confirms a transport-free consumer component; native peer admission,
host legality, settled effects/votes, checksums, rejoin, deployment, and release remain unverified.

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

The [Runtime-v3 bundle](../protocol-artifact/runtime-v3-gameplay/README.md) was copied byte-for-byte
from `AI-Ascension/sts2-protocol` main
`f2dac90529f584a6511c1760adce9da28f7f910a` (MIT) at artifact admission; current protocol main is
`d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404`. The canonical
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

The source-local `sts2-exo-bridge` and separately built `sts2-exo-executor` add a Linux,
standard/fresh single-turn lane under [ADR 0018](decisions/0018-exo-one-shot-executor-package.md).
They preserve the existing closed wire schema and expose a distinct one-shot descriptor with
`full_runtime_admission: false`. Map, expert, management/recovery, continuity and full episode
admission remain unsupported by this executable. Existing full preflight is unchanged.
Rollback removes the opt-in executable/configuration; no stored record migration is introduced.
The exact build and synthetic real-Exo lane are in the
[bridge README](../experiments/exo-agent/bridge/README.md).

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

Frozen Runtime-v2 decoder fields remain required even when their permitted value is null. The persistent
MCP child uses cancellable asynchronous pipes and joined supervisors, so shutdown/reaping is bounded,
errors remain visible, and inherited pipe handles cannot strand harness I/O workers. MCP and gateway
sessions are separate namespaces, and the configured MCP child receives both identities explicitly, so
its adapter must bind them without equating them. The six-tool Runtime-v3 catalog is independent of the
retained Runtime-v2 four-lane scheduler; configure the same explicit `STS2_MCP_SESSION_ID` in the
independently launched gateway and harness (harness, gateway and MCP default to `mcp-session-1`, and the
gateway session independently to `session-1`).

Dispatch preserves the complete host legal-action reference (`action_id` plus typed `action` payload)
across the MCP boundary. A bare payload is not a legal-action reference. Canonical schema regressions
cover end-turn and card payloads, including explicit nullable card targets.

## Owner lease acquisition under transient contention

[ADR 0029](decisions/0029-owner-lease-transient-busy-retry.md) makes `Lease::acquire` retry the
non-blocking `flock` attempt for a bounded interval (32 attempts, 5 ms apart) before reporting
`Busy`. An `flock` belongs to the open file description, so a descriptor this process has already
closed can still hold the lock while a spawned child has not yet reached `execve`. This is
`additive-compatible`: the owner-lock file, the locking primitive, mutual exclusion and release on
process death are unchanged, exhausting the attempts still fails closed with `Busy`, and no journal,
schema, range or bound changes. Only the reporting latency under genuine contention changes, and it
is now bounded by the same interval the map publication lock already uses: 32 attempts 5 ms apart,
about 160 ms nominal and 162-172 ms as measured, after which a contended acquisition still fails
closed with `Busy`. On the contended `create` path the caller's authority guard is held for the
length of that wait before the call fails.

## Runtime Exo admission gate

[ADR 0031](decisions/0031-runtime-exo-admission-gate.md) wires the ADR 0017 preflight and the
`ExoAdmittedTransport` envelope into the runtime-v3 transport seam. This is `breaking` for
operator configuration: `STS2_EXO_ADMISSION` now selects the admission mode, the reviewed
`envelope` mode is the default when it is absent, and it requires the complete operator-trusted
deployment identity (`STS2_EXO_PACKAGE_DIGEST`, `STS2_EXO_EXTENSION_DIGEST`,
`STS2_EXO_BRIDGE_DIGEST`, `STS2_EXO_MODEL_BINDING`, `STS2_EXO_PROVIDER`, `STS2_EXO_ENDPOINT`,
`STS2_EXO_PROMPT_DIGEST`, `STS2_EXO_TOOL_DIGEST`, `STS2_EXO_CONFIG_DIGEST`,
`STS2_EXO_NATIVE_INSTANCE_ID`, `STS2_EXO_MODEL_EXECUTION_ID`, `STS2_EXO_REQUEST_ID`,
`STS2_EXO_TURN_ID`), plus one required artifact locator, `STS2_EXO_PACKAGE_PATH`. A missing or
unverified deployment, or an absent or empty package locator, ends the run while settings are
assembled, before any gateway, MCP, provider or game effect, and no request bytes are emitted.
`envelope` inspects the exact bytes at `STS2_EXO_PACKAGE_PATH` and the exact bytes of the bridge
executable it is about to launch, and hashes both into the inspected identity, so a swapped package
is refused as `IdentityMismatch("package_digest")` and a swapped bridge as
`IdentityMismatch("bridge_digest")`. The inspected digests are always computed from the located
bytes; `STS2_EXO_PACKAGE_DIGEST` and `STS2_EXO_BRIDGE_DIGEST` are never substituted for the
observation. The identity comparison precedes the capability gate and `package_digest` is evaluated
first. The envelope now requires launch arguments `--run`, an absolute bridge configuration path,
and its digest. The shared bridge loader verifies that configuration, the executor, the reviewed
extension, Node and source revision. The package locator must resolve to that same executor.
Extension and prompt identity bind the complete reviewed extension source; tool identity uses the
reviewed tool catalog; model, provider route and configuration identity come from the verified
launch configuration. The instance identity comes from the gateway runtime configuration and is
subsequently subject to gateway allocation validation. It is not native acceptance evidence.
This is a breaking operator-configuration change, with unchanged wire schemas. It removes the
unbound extension obstacle but does not promote unverified cancellation/recovery capabilities;
those still prevent full admission until their runtime composition is verified.
`STS2_EXO_PACKAGE_PATH` is a
backward-incompatible addition to the reviewed envelope contract — every deployment that does not
supply it now fails closed with `STS2_EXO_PACKAGE_PATH is required`. The already-documented raw-wire
development bridges
(`docs/OLLAMA_MODEL_SELECTION.md`, `experiments/live-combat/README.md`) must set
`STS2_EXO_ADMISSION=legacy`, which is an explicit acknowledgement of an un-admitted bridge rather
than an admission. Rollback is to set `legacy`; no wire field, schema, contract version or durable
record changes. Per-turn envelope admission for a multi-turn episode remains open.
