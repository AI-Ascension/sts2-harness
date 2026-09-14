# Evidence: real pinned-Exo execution of the STS2 extension (2026-09-14)

Issue: AI-Ascension/sts2-harness#139. Tracker: #138. Stacks on the contract PR (#152).

Status: synthetic-process spike. A real provider, native instance, gameplay settlement, and the
production Rust bridge remain `unverified`.

## Selected path under test

`experiments/exo-agent/extension/src/index.ts` loaded through `agent.typescript.module_path`, the
single executor path frozen by ADR 0017: `defineHarness.runTurn` → `runResponsesHarnessTurn` →
`ResponsesRuntime`.

## Identities

| Field | Value |
|---|---|
| `exo_source_revision` | `b06869ab789dee3f80ca474b5fa89dbe47ccb859` |
| `exo_executable_digest` | `eef56bfb39f67f7c616284547ad7a8acf364f897a95ce91065e7174b966c46f8` |
| `extension_sha256` | `8321d446aba3860d3a80e26c6e176c15487fd011629f87311b6e5f30eac0945b` |
| `sts2_fixture_sha256` | `119daeaefad4897463afa2e83d238154d317f4781371fa23e34e7bf513900b41` |
| `sts2_driver_sha256` | `ab07ee8ce2df258ff35cdcd420963bc55fb85441b94c38e02514f8d8204c8abe` |
| Exo toolchain | Node `v22.15.0`, pnpm `10.26.2` |
| `model_binding` | `gpt-test` → bounded local synthetic endpoint (no real credential) |
| Sandbox | `local-process`, `--max-tool-round-trips 0` |

## Result

| Metric | Value |
|---|---|
| Model calls to the synthetic endpoint | 1 |
| Correlated `turn_id` | `01a09de4-c5fe-78b2-955b-975990fbc418` |
| Persisted terminal decision text | `{"decision":"wait","rationale":"synthetic spike decision"}` |
| Usage (prompt / completion) | 11 / 5 |

The model request carried the extension's `syntheticInstructions` system message. The driver asserts
exactly one model call, a correlated turn, and the decision text, and fails otherwise.

## What this establishes

The real pinned Exo TypeScript harness loads the selected extension, executes one turn through
`ResponsesRuntime`, and returns a correlated turn record with truthful usage and the terminal
decision text. This satisfies the synthetic-process half of #139 criterion 2 for the selected
extension.

## What this does not establish

Terminal `sts2.exo-decision-v1` admission, the `sts2-exo-bridge-wire-v1` envelope/correlation, a
native instance/package identity, a real provider, gameplay settlement, the production Rust bridge,
and non-Linux behavior. Those remain `unverified`.

## Reproduction

See [`experiments/exo-agent/spike/README.md`](../../experiments/exo-agent/spike/README.md). Raw
evidence from the recorded run: `report.txt`, `events.json`, `requests.jsonl`.
