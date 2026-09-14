# ADR 0017: Pinned STS2–Exo executor and bridge contract

## Status

Accepted as the harness-owned source/contract design for issue [#139](https://github.com/AI-Ascension/sts2-harness/issues/139).
The source review and deterministic wire/capability fixtures are `source-derived`. A real pinned
Exo executor run, native instance, package digest, model binding, and STS2 gameplay compatibility
remain `unverified`.

## Decision

The harness owns a narrow `sts2-exo-bridge-v1` contract behind `ExoTransport`. It owns request and
response bounds, strict framing, control-plane correlation, capability/preflight, source/package
manifest records, and evidence labels. It does not own Exo internals, a TypeScript runtime,
model credentials, a game adapter, or a native host.

The one selected executor path is a dedicated operator-owned TypeScript STS2 extension loaded at
`agent.typescript.module_path` by the existing `TypeScriptHarness`. Its only allowed Exo executor
path is:

```text
defineHarness.runTurn
  -> runResponsesHarnessTurn
  -> ResponsesRuntime.runTurn
  -> ResponsesRuntime.complete / completeStream
```

The extension is the owner of the STS2 prompt/tool registration and terminal-decision projection.
Its source/package/extension/bridge/model/prompt/tool/config/native identities are separate
fields in the manifest and trusted preflight. There is no CLI fallback and no HTTP executor path.
`GET /health` is only a service probe. Upstream `POST /request` exposes substrate protocol
primitives but no executor-turn operation. CLI `conversation send` is a human-facing prompt path;
none can satisfy this contract.

The extension must return one terminal `sts2.exo-decision-v1` object. The existing strict parser
remains authoritative for decision variants and fields; correlation is carried by the outer
bridge envelope and host control receipt, not appended to model JSON.

## Upstream source review

The immutable source record is
[`protocol-artifact/exo-bridge-v1/manifest.json`](../../protocol-artifact/exo-bridge-v1/manifest.json).
It freezes repository `https://github.com/exoharness/exo`, the old audit revision
`7801005e6a1ab77008a05dbba80e0a2a7a56e35d`, candidate
`b06869ab789dee3f80ca474b5fa89dbe47ccb859`, and candidate tree
`f1c155c8b9b1c2ee83a34a04189e64203536b3ab`. The nine commits in that range were reviewed:

| Commit | Effect | Why it is unrelated to an STS2 executor hook |
| --- | --- | --- |
| `83975441699c2b4c5f508499e24723b140b0ba07` | Firecracker reset API | sandbox backend |
| `36b2b810dda0f6dcd8be2d4880c8587d3998ff6d` | persistent Firecracker terminals | sandbox/process bridge |
| `d76bd0134d331328b915959d2cd1caeea49b01c2` | bounded/safely deleted snapshots | sandbox storage |
| `cc4461e2ff27786eab7d122c391db8bb21e663fb` | ExoChat WebSocket reconnect | website chat |
| `29ebbcd959c27b3cf2539ba80e8cf912f0a5d3fb` | lower Firecracker setup latency | sandbox backend |
| `a153bf662ad7428602c6d1499a942e6f5d2685fd` | sandbox identity and lifecycle tracing | sandbox/provider tracing |
| `87bfc53d2de72c4a4930115aa609d8987ce20b78` | image configuration/materialization | workflow and images |
| `c5c1963a3445417e64938c139da88edd9154075d` | sparse snapshots with cumulative bases | sandbox snapshots |
| `b06869ab789dee3f80ca474b5fa89dbe47ccb859` | guardian-tools deferred-script path | guardian tooling |

The range changes Firecracker, sandbox, website chat, workflow, or guardian code. It does not
add a machine executor-turn request, an STS2 extension, a bounded terminal-decision response, or
a cancellation/recovery hook. No upstream implementation is copied into this repository.

## Identity and trust

`ExoIdentity` has closed, independently validated axes:

| Axis | Meaning | Source of trust |
| --- | --- | --- |
| `source_revision` | immutable Exo source commit | reviewed manifest |
| `package_digest` | built Exo package/executable | operator hash of bytes |
| `extension_digest` | dedicated STS2 TypeScript extension | operator hash of source/package |
| `bridge_digest` | bounded process/transport bridge | operator hash of executable |
| `model_binding` | provider/model binding | trusted operator configuration |
| `prompt_digest` | exact extension/system prompt inputs | trusted operator configuration |
| `tool_digest` | exact registered tool catalog | trusted operator configuration |
| `config_digest` | runtime/bridge configuration | trusted operator configuration |
| `contract_version` | `sts2-exo-bridge-v1` | descriptor and manifest |
| `native_instance_id` | one native process/instance | runtime handoff |

Self-reported Exo metadata is descriptive only. A complete trusted preflight requires every
deployment axis, including the native instance identity, to be supplied independently. The
source-only checked-in descriptor intentionally leaves package, extension, bridge, model,
prompt, tool, config, and native values absent.

## Capability and preflight contract

`sts2.exo-capability-v1` is closed (`additionalProperties: false`) and contains these concrete
fields: `schema_version`, `contract_version`, `identity`, `decision_kinds`, `profile_support`
(`standard`, `map`, `expert`), `context_modes` (`fresh`, `continuity`), `platforms`, `limits`,
`lifecycle` (`cancellation`, `recovery`, `idempotency`, `graceful_eof`), and `evidence`
(`terminal_decision`, `turn_identity`, `event_usage`, `replay`). Unknown fields, duplicate list
members, missing standard/action/fresh/Linux support, wrong identities, and invalid limits fail
closed.

`ExoLimits` keeps independent bounds for `max_standard_request_bytes` (131072),
`max_map_request_bytes` (393443), `max_response_bytes` (8192), `max_event_bytes` (8192),
`max_turns` (1), `max_turn_time_millis` (120000), `max_concurrency` (1), and
`max_tool_round_trips` (0). A trusted request may lower a bound but may not exceed its
advertisement. Map and expert are `unverified` in the source descriptor even though their
schemas remain named; they are not admitted by standard preflight.

`preflight` is a pure function. It performs no transport exchange, model call, `/health` probe,
filesystem read, or game-host action and returns `model_calls = 0` on success. It compares all
identity axes, selected platform/profile/context, and each independent limit before an executable
deployment is admitted.

## Bounded wire and lifecycle

The selected outer wire is `sts2.exo-bridge-wire-v1`. Both envelopes are one bounded UTF-8 JSON
value with no newline framing and no trailing bytes.

| Envelope | Closed fields | Shape |
| --- | --- | --- |
| request | `wire_version`, `request_id`, `turn_id`, `request` | `request` is the unchanged STS2 decision request |
| response | `wire_version`, `request_id`, `turn_id`, `outcome`, `decision`, `error_code` | `decision` only for `decision`; no decision for `cancelled`; bounded code for `failed` |

Request and turn IDs are ASCII control identities (128-byte maximum) and must match before a
response is accepted. The host-only `ExoControlIdentity` additionally binds run, episode,
model-execution, agent, conversation, session, turn, and idempotency IDs. Those values are never
placed in model-visible prompts. One response is terminal; a second response, stale ID, wrong
version, or duplicate frame is rejected.

The parser rejects invalid UTF-8, empty/oversized frames, malformed JSON, trailing bytes,
duplicate keys at any nesting level, unknown envelope/request fields, invalid request shape,
non-terminal decisions, and invalid error codes. A decision outcome is re-serialized only after
strict validation by `parse_decision`; no correlation or transcript field is appended. Cancellation
and failure are host lifecycle outcomes, not decision enum variants. Process transport writes one
request then explicitly shuts down stdin (EOF), bounds stdout, and maps timeout, non-zero exit,
oversize, unavailable, and malformed outcomes to fail-closed errors. No retry or gameplay fallback
is implied.

## Run, turn, replay, and private-state mapping

| Harness record | Exo record | Rule |
| --- | --- | --- |
| run | operator run control record | never model-visible |
| episode | one conversation scope | one host episode maps to one Exo conversation |
| model execution | one supervised executor turn | request/turn IDs are outer control fields |
| request | one `beginTurn` input projection | fair-play observation and legal IDs only |
| action decision | one terminal decision object | host binds action ID to current catalog |
| event/usage evidence | event/usage sideband | evidence is retained only when independently observed |
| cancellation | host cancellation receipt | no synthetic decision is invented |
| recovery | same idempotency key and turn identity | unresolved outcome remains unknown until reconciliation |

Fresh context is the only source-advertised mode. Continuity, compaction, branch, and co-op need a
separate capability state and evidence. Private Exo conversation/session state belongs to Exo and
is not copied into harness trajectory records; the harness retains bounded control identities,
decision records, and labeled sideband evidence. Replays require the same contract, identity,
request bytes, action catalog, and idempotency tuple. Parsing a response or observing a turn does
not prove game effect, terminal gameplay, Victory, full-run completion, reproducibility, or
release compatibility.

## Migration and compatibility

All checked-in code/config/test pin consumers now use candidate
`b06869ab789dee3f80ca474b5fa89dbe47ccb859`; the prior value remains only as explicit source-review
history in the artifact and this ADR. `runtime_v3_settings.rs`, `sts2-harness-exo.rs`,
`runtime_v3_durable_support.rs`, `runtime_v4_expert_artifact.rs`, the process fixtures, telemetry
tests, and the example config are migration consumers.

Legacy `provider_revision` continues to validate an exact source revision for the old request
shape. New durable/config records add the separate identity axes and contract version; they must
not use `provider_revision` as a package, bridge, model, or prompt identity. Old readers that
cannot understand the closed descriptor, envelope version, or identity axes fail closed. Existing
generic `ExecutionFingerprint` storage remains compatible while Runtime-v3 config digest material
records the separate Exo identity object; a later durable-schema migration may promote those axes
to first-class columns.

## Platform matrix and live completion

| Platform/profile | Source contract | Native evidence | Admission |
| --- | --- | --- | --- |
| Linux x86_64 / standard / fresh | `confirmed` at harness parser/preflight layer | `unverified` | source descriptor only; complete trusted identity required |
| Linux x86_64 / map or expert | schema names only | `unverified` | rejected as unverified |
| Windows, macOS, other architectures | no declared capability | `unsupported` | rejected |
| continuity, compaction, branch, co-op | no declared capability | `unverified` | rejected |

The missing real spike is a concrete blocker, not a design claim. The minimal reproducer is an
operator-owned `defineHarness` module loaded by `agent.typescript.module_path` that receives one
synthetic STS2 observation, invokes the selected `runResponsesHarnessTurn` path against the
original synthetic model endpoint, and attempts to emit one correlated bridge envelope. The
candidate Exo source exposes `runTurn`/`complete` and substrate conversation events, but the
reviewed range has no bounded machine terminal-decision/EOF/cancellation hook. The harness cannot
verify this reproducer without the actual candidate package, extension build, bridge executable,
model endpoint, and native credentials. Therefore this ADR does not call the design executable.

Live completion requires the exact package/extension/bridge/model/prompt/tool/config/native
digests, a correlated structured decision and turn identity, event/usage evidence, cancellation
and recovery results, replay/idempotency evidence, and a recorded Linux matrix handoff. Native
STS2 integration, licensed builds, gameplay effects, Victory/full-run, co-op, compaction, and
release qualification remain `unverified` until those gates pass.
