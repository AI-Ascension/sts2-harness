# Generation and provenance

## Generator

The committed renderer is `tools/sts2-expert-state-package`. It is a Rust workspace tool using
only the repository's locked `serde_json` dependency. Its source paths and digest are recorded in
`data/build-manifest.json`; the digest is calculated from the ordered `sha256sum` records for the
committed `Cargo.toml` and `src/main.rs`. The input of record is
`docs/research/sts2-expert-state-package/data/states.json`; the output root is passed as its first
argument. The generator reads the 131 state records and writes one report and one Mermaid source
per state, plus two global Mermaid sources.

```sh
cargo run --locked --package sts2-expert-state-package -- docs/research/sts2-expert-state-package
```

The renderer is deterministic for a fixed input file and tool version. It does not claim to
discover game state, extract host objects, calculate hidden RNG, or validate target-build behavior.
Generated files contain a generator marker and repeat the requirements-baseline boundary.

The inventory materializer used to create the committed JSON/CSV baseline is not committed. The
manifest therefore records that source gap explicitly, hashes the committed inventory artifacts,
and treats the Rust tool as a reproducible report/diagram renderer rather than claiming full
package regeneration from the supplied attachment.

## Input provenance

| Input | Origin | Status |
| --- | --- | --- |
| `data/states.json` | normalized from the supplied 2026-09-03 research specification | synthetic/proposed requirements baseline |
| `data/build-manifest.json` | build-pinned research ledger and public source references | source-derived fields plus unresolved local fields |
| `fixtures/` source hash | supplied specification SHA-256 `f7860c889a07ad03316db8c7f61c4cfe4f34b8947ba284258fec09a78f3b5c1b` | provenance marker; not a live capture |

No proprietary game assembly, PCK, executable, save, profile, screenshot, model output, provider
response, or privileged runtime label is an input to this package. Public source URLs and their
evidence labels remain in `data/source-ledger.csv` and the parent research report.

## Re-generation and review

After changing `data/states.json`, run the generator and the package checks from `ACCEPTANCE.md`.
Review the diff for state-ID churn, accidental evidence upgrades, raw UI operations, privileged
paths, and generated files outside `report/` or `diagrams/`. Generated output is review material;
it is not an excuse to omit a source or provenance record.
