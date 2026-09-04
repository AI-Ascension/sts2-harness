# Release Policy and Procedure

This repository currently contains foundation policy, documentation, and a non-released preparation
package. A release is a deliberate, immutable, evidence-backed publication; preparing a candidate,
publishing it, and verifying it are separate states.

## Authority

Only an explicitly authorized maintainer may approve publication. Contributors and agents may
prepare or verify a candidate when asked, but must not create or move tags, publish releases,
upload public artifacts, or deploy without explicit authorization.

## Version model

Repository, trajectory/schema, scoring, training/dataset, provider profile, gateway API, MCP
revision, and game-host versions are independent facts. A release must identify every version that
can affect an experiment, episode, score, replay, dataset, or artifact. Do not infer one version from
another.

Use Semantic Versioning for repository releases. Protocol or record changes are classified as
additive, deprecated, safety correction, or breaking in [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md).

## Readiness

A candidate is not release-ready until:

- the exact intended source commit is identified and reviewed;
- policy, formatting, lint, test, conformance, and dependency checks pass;
- serialized records and artifact manifests have deterministic fixtures and hashes;
- provenance, licenses, retention, redaction, and artifact allowlists are current;
- provider and gateway behavior is tested with approved fakes or exact authorized environments;
- any claimed runtime or model/provider compatibility has exact evidence; and
- known unverified boundaries do not make the release unsafe or misleading.

The current target has no released product artifact, provider result, game launch, or live runtime
claim.

## Prepare and verify

Run:

```bash
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

Build from the exact approved source revision. Inspect packaged contents and verify checksums before
publication. Exclude source-control metadata, build output, credentials, prompts/model data not
approved for distribution, valued saves, proprietary host files, personal paths, and unrelated
debug output. Do not rebuild different bytes during promotion.

Before any promotion, validate `patch-manifest.schema.json` and review the quarantined build
manifest at `docs/evidence/runtime-v3-preparation/data/build-manifest.json`. Run the bounded
`tools/patch-diff` utility against the exact base and candidate manifests, then attach separate
hashes and evidence for build, data, UI, action-catalog, and schema changes. A source diff or a
successful compile is not a target-build or runtime-compatibility result.

Promotion requires independent evidence for the host package, native package, gateway/MCP/harness
configuration, Exo revision, fair-play leak tests, stale/recovery behavior, setup-to-terminal
full-run traces, two-to-four-peer co-op traces, cleanup, clean-install replay, and rollback. Any
missing item keeps the candidate quarantined.

## Post-release and failure

Download and verify the published bytes in a clean location. Check manifests, licenses, record
schemas, representative offline replay/scoring behavior, and all stated compatibility facts. Record
post-release verification separately from publication.

Do not rewrite a tag or silently replace an artifact. Mark a defective release, preserve diagnostic
evidence without exposing private data, and publish a corrective version through the same gates.
