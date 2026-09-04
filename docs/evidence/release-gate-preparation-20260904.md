# Release-gate preparation — 2026-09-04

## Status

Evidence level: `source-derived` and `unverified`; candidate status: `quarantined`.

This record covers source changes prepared across the six STS2 repositories. It is not a release,
package, host-compatibility claim, or live Exo result. No repository was committed, pushed,
installed, deployed, launched, or published by this preparation pass.

## Recorded lineage

The working-tree baseline revisions and repository ownership are recorded in the aggregate
`ORCHESTRATION_MANIFEST.md`. Runtime-v3 gameplay schema digest is
`fbfb18279b0c7ebb350ef0ce0d56547fa11e83985b13380cb2b0f1dba4cb56e9`; co-op gameplay schema digest
is `2c34d013315fbf2e16de03dbe2bd4c43d4c13c744292548cc46ea960af5e1fa2`. The build manifest is
[`build-manifest.json`](../research/sts2-expert-state-package/data/build-manifest.json), and its
shape is checked by [`patch-manifest.schema.json`](../../patch-manifest.schema.json).

## Gate matrix

| Gate | Result | Evidence or blocker |
|---|---|---|
| Neutral Runtime-v3 schema/artifact | `source-derived` | Schema, manifest, goldens, checksums, and conformance source are present. |
| Typed core calculators/simulation | `source-derived` | Pure modules and deterministic parity tests are present; Rust execution is unavailable here. |
| Host/mod semantic boundary | `source-derived` | Rust contract, bounded queue, managed projection, postcondition and recovery seams are present. |
| Gateway lifecycle/fencing | `source-derived` | Supervisor, lease, Runtime-v3 route, and co-op session seams are present. |
| MCP/Exo semantic surface | `source-derived` | Six Runtime-v3 tools, strict projection, redaction, and no-fallback adapter tests are present. |
| Full-run and terminal routing | `source-derived` | `crates/harness/tests/full_run.rs` covers all declared stages and separate terminal states. |
| Co-op two-to-four-peer runtime | `unverified` | Deterministic coordination seams exist; no licensed multiplayer host trace is available. |
| Exact managed/native/game package build | `unverified` | Target game assemblies and package outputs are unavailable and were not copied. |
| Live Exo/provider trace | `unverified` | No provider credentials or live service call was made. |
| Six repository local gates | `unverified` | Required `cargo` commands cannot run because no Rust toolchain is installed. |
| Release promotion/rollback | `quarantined` | Requires exact hashes, clean-install replay, rollback, cleanup, and independent review. |

## Promotion rule

Keep the candidate quarantined until every unverified row has an exact input/configuration lineage,
bounded trace, independent oracle, package hash, cleanup result, and reviewer sign-off. Inferred
target-game rules must not be promoted to observed facts.
