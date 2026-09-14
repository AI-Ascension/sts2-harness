# STS2 Exo real-execution spike

Source-only evidence for the real pinned-Exo executor running the selected TypeScript extension
from [`../extension/src/index.ts`](../extension/src/index.ts). This is an operator-run spike; the
repository's Rust runtime does not invoke it.

## Contents

- `synthetic_model.rs` — original bounded synthetic model endpoint (no upstream or proprietary
  content). It serves `POST /responses` and `POST /chat/completions`, records each request (path and
  body), and returns one canned assistant message plus usage. It is not a Cargo target; the driver
  compiles it with `rustc`.
- `run_extension_spike.sh` — copies the extension into an operator-owned Exo candidate checkout,
  runs one turn through `exo --harness typescript --module ... `, and asserts exactly one
  `/responses` model call with a correlated turn id and the decision text. The binding uses model
  `o3-pro` so runtime selection reaches the Responses path (`ResponsesRuntime.complete`) selected by
  ADR 0017. The driver also sets `EXO_LITELLM_PRICES_PATH` to a non-existent file to suppress the
  model-runtime pricing fetch, so the spike makes no network calls.

## Run

Requires an Exo candidate checkout with the pinned revision, a built `exo` binary, and Node/pnpm to
run the TypeScript loader. Output must go outside the repository.

```sh
EXO_BIN=/path/to/exo \
EXO_SRC_ROOT=/path/to/exo-candidate \
NODE_BIN_DIR=/path/to/node/bin \
SPIKE_OUT=/tmp/sts2-exo-spike \
bash run_extension_spike.sh
```

The driver writes `report.txt` (identities and steps), `events.json` (conversation event log), and
`requests.jsonl` (synthetic endpoint requests). It exits non-zero if the run does not produce exactly
one model call, a correlated turn, and the decision text.

## Scope

This proves a synthetic-process executor run only. It does not prove terminal `sts2.exo-decision-v1`
admission, bridge envelope correlation, a native instance, a real provider, or gameplay.
