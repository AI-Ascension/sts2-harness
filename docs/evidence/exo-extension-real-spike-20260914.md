# Evidence: real pinned-Exo execution of the STS2 extension (2026-09-14)

Issue: AI-Ascension/sts2-harness#139. Tracker: #138. Stacks on the contract PR (#152).

Status: synthetic-process spike. A real provider, native instance, gameplay settlement, and the
production Rust bridge remain `unverified`.

## Selected path under test

`experiments/exo-agent/extension/src/index.ts` loaded through `agent.typescript.module_path`, the
single executor path frozen by ADR 0017: `defineHarness.runTurn` → `runResponsesHarnessTurn` →
`ResponsesRuntime.complete`. The binding uses model `o3-pro` so `runtimeFromModelBinding` selects
the Responses runtime rather than the chat-completions runtime.

## Identities

| Field | Value |
|---|---|
| `exo_source_revision` | `b06869ab789dee3f80ca474b5fa89dbe47ccb859` |
| `exo_executable_digest` | `eef56bfb39f67f7c616284547ad7a8acf364f897a95ce91065e7174b966c46f8` |
| `extension_sha256` | `8321d446aba3860d3a80e26c6e176c15487fd011629f87311b6e5f30eac0945b` |
| `sts2_fixture_sha256` | `119daeaefad4897463afa2e83d238154d317f4781371fa23e34e7bf513900b41` |
| `sts2_driver_sha256` | `ff71d2d7246bf4664d6492d91bf1b41fc40657a3568074c1af0850a797c1eb02` |
| `model_binding` | `o3-pro` → bounded local synthetic endpoint (no real credential) |
| Toolchain | Node `v22.14.0` (matches the extension pin); the loader runs `node --import tsx` |
| Sandbox | `local-process`, `--max-tool-round-trips 0` |

## Result

| Metric | Value |
|---|---|
| `/responses` calls to the synthetic endpoint | 1 |
| `/chat/completions` calls | 0 |
| Correlated `turn_id` | `01a09df3-7a43-7483-b48b-1f2dae7cb6ad` |
| Persisted assistant decision text | `{"decision":"wait","rationale":"synthetic spike decision"}` |
| Usage (prompt / completion) | 11 / 5 |

The model request carried the extension's `syntheticInstructions` system message. The driver asserts
exactly one `/responses` call, zero chat-completions calls, a correlated non-null turn id, and the
decision text, and fails otherwise.

## What this establishes

The real pinned Exo TypeScript harness loads the selected extension, executes one turn through the
`ResponsesRuntime` path selected by ADR 0017, and returns a correlated turn record with truthful
usage and the assistant decision text. This is the synthetic-process half of #139 criterion 2 for the
selected extension.

## What this does not establish

The persisted text is the raw assistant message; it has **not** been admitted through
`parse_decision` / `sts2.exo-decision-v1`. This spike also does not establish the
`sts2-exo-bridge-wire-v1` envelope/correlation, a native instance/package identity, a real provider,
gameplay settlement, the production Rust bridge, or non-Linux behavior. Those remain `unverified`.

## Reproduction

See [`experiments/exo-agent/spike/README.md`](../../experiments/exo-agent/spike/README.md). Raw
evidence from the recorded run: `report.txt`, `events.json`, `requests.jsonl`.
