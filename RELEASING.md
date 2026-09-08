# Release Policy and Procedure

This repository currently contains a non-released preparation package and a bounded runtime
coordinator. A release is a deliberate, immutable, evidence-backed publication; preparing a
candidate, publishing it, and verifying it are separate states.

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

Current evidence includes the [dated bounded runtime-v1 host trace](docs/evidence/runtime-v1-host-integration-20260902.md),
the [Windows Astra runtime-v3 campaign and replay](docs/evidence/seeded-astra-campaign-20260906.md),
and the [Linux Astra runtime-v3 campaign and replay](docs/evidence/linux-seeded-campaign-20260906.md).
The Windows record documents an uninterrupted setup-to-defeat campaign followed by a fresh complete
replay; the Linux record documents a campaign continued across one controller restart followed by a
fresh complete replay. They preserve provider-backed decisions and settled operations for the named
STS2 v0.107.1 fixtures, exact downstream artifacts, and recorded configurations. They do not
establish a released artifact, a model-played victory, complete campaign or state coverage, native
multiplayer, or broader compatibility.

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

The current runtime coordinator is a bounded one-instance executable. Its deterministic coordinator,
replay, record, and artifact-lineage libraries are not all assembled into that executable, and the
runtime-v2 lane remains a fake-boundary check. Provider execution and the named runtime-v3 campaigns
have recorded evidence, while final current-head release artifacts, full native Runtime-v4
legality/effects, model-played victory, complete campaign coverage, co-op actuation, release
packaging, deployment, and compatibility beyond the recorded host remain unverified.

## Post-release and failure

Download and verify the published bytes in a clean location. Check manifests, licenses, record
schemas, representative offline replay/scoring behavior, and all stated compatibility facts. Record
post-release verification separately from publication.

Do not rewrite a tag or silently replace an artifact. Mark a defective release, preserve diagnostic
evidence without exposing private data, and publish a corrective version through the same gates.
