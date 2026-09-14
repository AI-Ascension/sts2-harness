# ADR 0017: STS2–Exo executor boundary and bridge contract

- Status: Proposed (this PR); accepted on merge
- Date: 2026-09-14
- Supersedes/extends: [ADR 0006](0006-exo-full-run-and-evidence-gates.md) (process seam only)
- Tracker: AI-Ascension/sts2-harness#138; delivery: #139

## Context

The harness has an Exo adapter seam (`ExoTransport`, `ExoProcessTransport`, `ExoConfig`) but no
production path that executes the real Exo runtime. The checked-in example pins the 2026-09-02 audit
revision `7801005e6a1ab77008a05dbba80e0a2a7a56e35d` and a placeholder endpoint. The current reviewed
candidate is `b06869ab789dee3f80ca474b5fa89dbe47ccb859` (nine commits ahead; drift, not a proven
break). A TOML edit alone cannot enable it: runtime admission separately enforces a reviewed revision,
and no checked-in bridge executes Exo.

This ADR freezes the executable adapter contract before the production bridge (#141) is written. It
is source-derived from upstream Exo at `b06869ab789dee3f80ca474b5fa89dbe47ccb859` and harness at
`4ddcfd5`. Live Exo/model/gameplay behavior remains `unverified` until the #149 handoff.

## Decision

### 1. Selected execution path

A dedicated, operator-owned **bounded machine bridge process** is the adapter behind
`ExoTransport`. It embeds the reviewed Exo executor as a library and exposes exactly one JSON
request on stdin and one bounded JSON decision on stdout. The harness never invokes a shell and keeps
its existing `ExoProcessTransport` direct-child supervisor, isolation, timeout, and byte bounds
(ADR 0006).

Inside that bridge, the selected machine interface is the upstream **executor embedding API**, not an
HTTP or CLI surface:

- Invoke one turn: `HarnessConversation::send(SendRequest { input, session_id }) -> SendResult`
  where `SendResult { session_id, turn_id, latest_event_id }`.
  Source: upstream `crates/executor/src/harness_types.rs:56-58`,
  `crates/executor/src/executor_types.rs:231-243`.
- Read correlated evidence: `ConversationHandle::get_events(EventQuery)` filtered by `turn_id`.
  Terminal decision = last assistant `EventData::Messages { messages, response_id, usage }`;
  usage rides the same event. Public exports: `crates/executor/src/lib.rs:50-96`.
- Real Exo harness binding: `TypeScriptHarness::<ExoToolRuntime>::exo_from_root(...)` with
  `CreateAgentRequest.harness = AgentHarnessKind::Exo` and
  `typescript.module_path = "exo/harness.ts"`.
  Source: `crates/executor/src/typescript.rs:875-900`; `harness_types.rs:60-76`.

Rationale: this is the same code path the upstream CLI and scheduler use, it returns an explicit
`turn_id`/`latest_event_id` for correlation, and usage arrives on the structured event. It is
preferable to parsing human-rendered CLI output. The bridge may alternatively use
`exo conversation send` followed by `exo conversation events` (structured) only if embedding is
infeasible on a supported platform; `send` alone is insufficient because it emits no turn identity
(`crates/cli/src/main.rs:2332-2345`, `:2254-2288`).

### 2. Rejected interfaces

- `GET /health` is a liveness string only; it is not readiness to infer
  (`crates/exoharness/src/http/server.rs:59,71-73`).
- `POST /request` is the exoharness durable substrate transport, not an executor/model API
  (`exoharness/docs/http.md:1-3`); it contains no send/execute operation.
- `conversation_begin_turn`/`turn_add_events`/`turn_finish` record durable turn data without
  calling a model (`crates/exoharness/src/protocol.rs:225-311`).
- A fake executor, direct Astra/Codex call, direct provider HTTP call, or synthetic provider result
  cannot satisfy the real-executor criterion.

### 3. Independent identities (never one `revision`)

The bridge manifest and every durable record keep separate fields:

| Field | Meaning |
|---|---|
| `exo_source_revision` | Git commit of the reviewed Exo source |
| `exo_executable_digest` | SHA-256 of the built bridge/Exo package bytes |
| `sts2_bridge_digest` | SHA-256 of the STS2 extension/bridge artifact |
| `model_binding_id` | Operator model binding identifier (not a credential) |
| `prompt_digest` / `tool_digest` / `config_digest` | Canonical digests of effective inputs |
| `contract_version` | This bridge wire contract version |
| `instance_id` | Native instance identity, namespaced separately |

Self-reported version strings are not authenticated provenance; injected metadata must be bound to
trusted operator configuration. This corrects the current overloaded use of `revision` in
`config.example.toml` and `runtime_v3_settings.rs`.

### 4. Capability and preflight descriptor

A closed, versioned descriptor declares schema versions, decision kinds, standard/map/expert
support, context/continuity modes, supported platform, independent byte/turn/time/concurrency
limits, and lifecycle/recovery support plus evidence status. Preflight must not call the model.
Unknown, malformed, swapped, or unsupported descriptors fail closed before any model or game effect.
Closed-schema changes require explicit reader/settings/fingerprint migration.

### 5. Wire mapping, framing, and bounds

Preserve existing harness bounds:

- standard request ≤ 131,072 bytes; map request ≤ 393,443 bytes; decision ≤ 8,192 bytes;
  exchange deadline ≤ 120,000 ms.
- lower-layer MCP/map envelope limits remain authoritative; do not silently append fields that
  `parse_decision` rejects.
- UTF-8 JSON, one request per invocation, exactly one terminal decision, explicit error shape,
  duplicate/unknown fields rejected, identity correlation by `request_id`/`turn_id`, cancellation
  and EOF semantics defined. Any larger metadata/control channel needs its own version and bounds.

### 6. Run/episode/model-execution and Exo object mapping

- Harness `run_id`/`episode_id`/`model_execution_id` are bound to Exo agent/conversation/session/
  turn/event identifiers by an explicit mapping record; identifiers stay in their own namespaces
  (per AGENTS.md).
- Replay/idempotency: a recorded episode replays decisions without any Exo/model call; a repeated
  invocation with the same `request_id` must not create a second model effect.
- Private state: Exo conversation history, prompts, secrets, and memory are operator-owned private
  state and never become model-visible game data. Control-plane identity and credentials are
  excluded from the fair-play projection.

### 7. Platform matrix

Initial supported matrix is Linux only. Windows/macOS Exo execution is not claimed from Linux tests.
The bridge's descendant containment is the operator's responsibility (ADR 0006); only the direct
child is supervised.

### 8. Live completion criteria (integration-specific)

Integration completion requires one terminal gameplay episode with truthful Exo lineage and a fresh
replay with zero model calls. A terminal Defeat is a valid integration outcome. This is distinct
from broader Victory/full-run promotion and from optional compaction/branch/co-op capabilities.

## Pin and migration inventory

All pin/schema consumers were inventoried at `4ddcfd5`. Files that must change together on a pin or
schema bump:

- Pin: `crates/harness/src/bin/sts2-harness-exo.rs:10`,
  `crates/harness/src/bin/runtime_support/runtime_v3_settings.rs:13`,
  `experiments/exo-agent/config.example.toml:3`, and the test fixtures
  (`tests/support/runtime_v4_executable_composition_fixture.rs:20`, `tests/runtime_startup.rs:19`,
  `tests/completed_resume_process.rs:19`, `tests/exo_adapter.rs:110,137,203`,
  `tests/exo_episode.rs:112`, `tests/action_plans.rs:41`, `tests/context_capture.rs:124`,
  `tests/context_control.rs:14`, `tests/context_control_races.rs:7`, `tests/provider_redaction.rs:13`,
  `runtime_v3_telemetry_tests.rs:25`, `runtime_v3_telemetry_identity_privacy_tests.rs:51,114`).
- Schema: `crates/harness/src/exo/protocol.rs:22-31`,
  `crates/harness/src/exo/protocol/request.rs:168-172`,
  `crates/harness/src/exo/protocol/request_validation.rs:47-49`,
  `crates/harness/src/bin/sts2-astra-bridge.rs:272`,
  `crates/harness/src/context_control/render.rs:333`, `crates/harness/src/episode/map.rs:16-20`.
- Durable fingerprint: `runtime_v3_durable_support.rs:44,77-81` feeds `ExecutionFingerprint`;
  `crates/harness/src/worker_runtime_store.rs:193-195` fails closed on mismatch, so any revision/
  schema/config change invalidates existing durable episodes and requires an explicit store
  migration/version decision.
- Docs: ADR 0004/0006/0007, `docs/COMPATIBILITY.md`, `experiments/exo-agent/README.md`,
  `README.md`, `docs/OLLAMA_MODEL_SELECTION.md`, `CHANGELOG.md`, `RELEASING.md`.

The checked-in `config.example.toml` `decision_schema` key and `endpoint` placeholder are not parsed
by code; they must be reconciled when the production bridge lands.

## Consequences

- Positive: one concrete, source-referenced execution path; explicit identity separation; fail-closed
  preflight; existing byte bounds retained.
- Cost: an operator-owned bridge artifact and an Exo toolchain (Node 22/pnpm 10 per upstream
  `mise.toml`); durable-episode migration on any pin/schema change.
- Risk: `BasicHarness` hard-codes the OpenAI Responses API, so the synthetic spike must use the
  TypeScript Exo harness or an explicit provider-format override (upstream change required otherwise).
  A synthetic endpoint must receive a non-empty key because the client requires one.

## Evidence

- Source-derived: selected executor API, rejected interfaces, routing behavior, and pin inventory
  above, cited to upstream `b06869a` and harness `4ddcfd5`.
- Confirmed: upstream HEAD equals the tracked candidate; no production Exo bridge exists.
- Unverified: real pinned Exo execution against a synthetic model; live provider; native gameplay;
  Windows/macOS behavior.

## Open items gating issue closure

- Run the bounded real-Exo + synthetic-model spike and record exact source/package/extension
  identities (acceptance criterion 2).
- Add contract schema files/goldens and contract vectors covering all semantic decisions, ordinary/
  map bounds, wrong correlation, incompatible schema versions, and unavailable capabilities.
- Implement preflight negative tests for malformed/unknown/swapped/wrong-revision inputs.
