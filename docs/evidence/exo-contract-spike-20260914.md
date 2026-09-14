# Evidence: STS2-Exo executor contract spike (2026-09-14)

Issue: AI-Ascension/sts2-harness#139. Tracker: #138.

Status: synthetic-process spike complete. Live provider, native gameplay, and end-to-end
integration remain `unverified`.

## What this proves

The pinned real Exo implementation executes one model turn against an original bounded synthetic
model endpoint and returns a correlated turn record with truthful usage. Both machine paths behind
`HarnessConversation::send` were exercised:

- the Rust `Basic` executor, and
- the canonical TypeScript Exo harness (`exo/harness.ts`).

This is a synthetic-process result only. It does not prove STS2 decision semantics, the production
bridge, native gameplay, a real provider, or non-Linux behavior.

## Identities

| Field | Value |
|---|---|
| `exo_source_revision` | `b06869ab789dee3f80ca474b5fa89dbe47ccb859` |
| `exo_binary_sha256` | `eef56bfb39f67f7c616284547ad7a8acf364f897a95ce91065e7174b966c46f8` |
| Exo toolchain | Node `v22.15.0`, pnpm `10.26.2` |
| Harness base | `4ddcfd5` |
| Synthetic fixture | `experiments/exo-agent/spike/synthetic_model.py` (original) |
| Contract version | `sts2.exo-bridge-spike-v0` |

The built `exo` binary is a debug artifact of the pinned source; its digest binds the exact package
inspected by this spike. A different build changes the digest.

## Results

| Path | Harness | `turn_id` | Model calls | Usage (prompt/completion) |
|---|---|---|---|---|
| basic | Rust `Basic` | `01a09d76-4ad1-7783-a299-afde97bd2e41` | 1 | 11 / 5 |
| exo | TypeScript `exo/harness.ts` | `01a09d76-750d-76f3-8835-4c0367bc5a41` | 1 | 11 / 5 |

In both runs the assistant message persisted in the conversation event log carried the synthetic
text and the `synthetic-model` usage record. Exactly one model call reached the synthetic endpoint
per turn; there were no retries and no direct provider contact.

## Reproduction

Requires a built binary from the pinned Exo source and Node 22.15.0/pnpm 10.26.2 to install and run
the TypeScript harness. Output must go outside the harness repository.

```bash
EXO_BIN=/path/to/pinned/exo \
EXO_SRC_ROOT=/path/to/pinned/exo-source \
NODE_BIN_DIR=/path/to/node22/bin \
SPIKE_OUT=/tmp/sts2-exo-spike \
bash experiments/exo-agent/spike/run_spike.sh
```

The runner starts `synthetic_model.py`, creates a temporary Exo root, registers a synthetic model
binding at `http://127.0.0.1:<port>`, creates a `local-process` agent with `--max-tool-round-trips 0`,
sends one prompt, and writes `<mode>-report.txt`, `<mode>-events.json`, and
`<mode>-requests.jsonl`.

## Wire observations

- The Rust `Basic` executor called `POST /responses`.
- The TypeScript Exo harness selected `POST /chat/completions` for a non-Anthropic, non-OpenRouter
  model, matching the routing in `exoharness/typescript/model-runtime/responses.ts`. This confirms
  that the bridge model binding must declare its provider format rather than assume Responses.

## Evidence states

- `confirmed`: the pinned Exo binary executes one real executor turn in both harness modes against
  the synthetic endpoint, with a correlated `turn_id`, usage, and exactly one model call.
- `source-derived`: the selected machine interface and rejected interfaces in
  [ADR 0017](../decisions/0017-sts2-exo-executor-bridge-contract.md).
- `unverified`: STS2 semantic decision production, the production bridge, real provider calls,
  native gameplay settlement, and Windows/macOS execution.
- `unsupported`: any claim that this spike proves model-played gameplay or Victory.
