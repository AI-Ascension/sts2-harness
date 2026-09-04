# Release-gate preparation — 2026-09-04

## Status

Evidence level: `source-derived` and `unverified`; candidate status: `quarantined`.

This record covers source changes prepared across the six STS2 repositories. It is not a release,
package, host-compatibility claim, or live Exo result. The implementation was committed and
pushed to six open PR branches; no package was installed, deployed, launched, or published.

## Recorded lineage

The working-tree baseline revisions and repository ownership are recorded in the aggregate
`ORCHESTRATION_MANIFEST.md`. Runtime-v3 gameplay schema digest is
`b37c80f583aeaf4f81ede2083bcfb4129196baf5eb092470e8738173c4b7226c`; co-op gameplay schema digest
is `2c34d013315fbf2e16de03dbe2bd4c43d4c13c744292548cc46ea960af5e1fa2`. The build manifest is
[`build-manifest.json`](runtime-v3-preparation/data/build-manifest.json), and its
shape is checked by [`patch-manifest.schema.json`](../../patch-manifest.schema.json).

The six PR heads and their GitHub CI/policy results are recorded in the aggregate manifest. Those
remote checks validate the repository workflows; they do not substitute for the unavailable
licensed target build, live Exo connection, or runtime traces below.

Review follow-up: the current harness consumer pin above is refreshed to protocol commit
`82507361890c1bdce6cffeaf7e616d93e53a7d99`. The original preparation baseline used
`fbfb18279b0c7ebb350ef0ce0d56547fa11e83985b13380cb2b0f1dba4cb56e9`, preserved in the build
manifest's base row. The [consumer artifact README](../../protocol-artifact/runtime-v3-gameplay/README.md)
records exact provenance and bounded test coverage. The gate matrix below is the historical
preparation record, not current review test results. Refreshing this consumer pin alone does not
establish an integrated six-repository candidate or advance runtime promotion.

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
