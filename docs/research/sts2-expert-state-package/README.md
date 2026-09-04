# STS2 expert-state research package

This package is a generated, proposed requirements baseline for the fair-play autonomous
*Slay the Spire 2* harness described in the dated parent report. It is owned by `sts2-harness` as
experiment, planning, replay, evaluation, and artifact-lineage material. It does not add a game
adapter, host authority, MCP/gateway implementation, simulator runtime, or production support claim.

## Package contents

| Directory | Contract | Current artifact status |
| --- | --- | --- |
| `data/` | Build manifest, state/field/action/transition inventories, matrices, ledgers | Generated; counts checked |
| `schemas/` | Five closed JSON Schema Draft 2020-12 contracts | Generated; production observation is fair-play projected |
| `fixtures/` | Synthetic normal, boundary, adversarial, recovery, and patch-regression JSONL | Generated; 655 records |
| `report/` | One Markdown requirements record per candidate state | Generated; target-build validation required |
| `diagrams/` | Global and per-state Mermaid source | Generated; PNG rendering/visual inspection remains external |

The package baseline contains 131 candidate states, 4,315 observation rows, 421 state/action rows,
and 1,059 transition rows. It also contains five fixture records per state, for 655 synthetic
fixtures total. These are package counts, not claims that those states or fields were observed in a
licensed target executable.

The per-state records and Markdown pages carry the policy-facing required, on-demand, historical,
derived, estimated, and denied field sets. The exhaustive state-by-field join is preserved in
`data/state-field-matrix.csv` and the normalized row-level field records in `data/observations.json`.

## Evidence and fair-play boundary

All generated records are labeled `proposed`, `hypothesized`, `required`, or synthetic as applicable.
No exact target-build discovery campaign, expert panel, field ablation, simulator/live parity run,
cooperative live run, screenshot census, PNG inspection, or PDF build is represented as complete.
The production observation schema has no generic `privileged_fields` path. Hidden RNG, unrevealed
outcomes, unrevealed map content, private teammate information, and other non-player-legitimate
values remain denied or offline-only.

The 131-state package is intentionally distinct from the earlier 144-state architecture inventory
in the parent report: the report uses uppercase dotted IDs, while this package uses lower-snake
candidate IDs from a separate supplied specification. Neither is a measured census. There is no
accepted one-to-one mapping or evidence that the difference is exactly 13 missing game states.
The 144/131 inventories must remain separate until an explicit ID/meaning mapping is reviewed;
target-build discovery is then needed to validate that mapping against actual host behavior.

The 33 observation templates are expanded over states (32 occur 131 times and one 123 times),
not 4,315 independently discovered fields. The 655 fixture envelopes similarly repeat five safety
scenarios and do not execute the 421 candidate actions or validate 1,059 host transitions. See
[`ACCEPTANCE.md`](ACCEPTANCE.md) for the exact structural and behavioral evidence boundary.

## Reproducibility

The package is generated from `data/states.json` by the Rust-owned tool in
`tools/sts2-expert-state-package`. From the repository root:

```sh
cargo run --locked --package sts2-expert-state-package -- docs/research/sts2-expert-state-package
```

This rewrites only generated report and Mermaid outputs. It does not access game files, saves,
credentials, providers, network services, or unrevealed runtime state. See
[`GENERATION.md`](GENERATION.md) for provenance and [`ACCEPTANCE.md`](ACCEPTANCE.md) for the current
verification boundary.
