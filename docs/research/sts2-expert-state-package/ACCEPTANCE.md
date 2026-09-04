# Package acceptance record

This is the current acceptance ledger for the generated requirements package. “Generated” means
the artifact exists and its structural contract is checked; it does not mean the target game was
observed or that an autonomous harness is production-ready.

## Structural gates

| Artifact or invariant | Status | Evidence required/available |
| --- | --- | --- |
| 131 unique candidate states | generated and checked | `data/states.json` record count and exact supplied-ID comparison |
| 4,315 observation rows | generated and checked | `data/observations.json` record count |
| 421 state/action rows | generated and checked | `data/actions.json` record count |
| 1,059 transitions | generated and checked | `data/transitions.json` record count |
| Five JSON Schemas | generated | closed roots, namespaced IDs, provenance, freshness, settlement, and quarantine fields |
| 655 synthetic fixtures | generated and checked | 131 states x 5 fixture classes; no live capture claim |
| Per-state Markdown | generated | 131 state files plus report index |
| Mermaid sources | generated | 131 state diagrams plus two global diagrams |
| Committed renderer digest | generated and checked | `data/build-manifest.json` binds the Rust renderer source paths and digest |
| Inventory artifact digests | generated and checked | `data/build-manifest.json` hashes the committed JSON/CSV members |
| Fair-play production path | specified | no generic privileged field; denied values do not enter policy schemas |
| Exact target-build hashes | unresolved | licensed local installation and legal hash record required |

The committed Rust package tests now enforce inventory joins, every per-state renderer output,
inventory and renderer digests, and all 655 synthetic envelope records. Negative mutations verify
that dangling/cross-state references, generation drift, forged targets, admitted privileged fields,
and upgraded evidence labels fail. JSON Schema validation is a separate lane; these envelope
fixtures are not instances of the five namespaced schemas and do not exercise runtime safety.
Separate full schema instances and 25 negative mutations now run under Draft 2020-12 validation
in the ordinary Rust workspace tests. Schema and fixture bytes are also digest-bound in the build
manifest. See [`fixtures/schema/README.md`](fixtures/schema/README.md) for the exact 30-case scope.

The 655 records are five repeated safety templates indexed by state name. Their generation/input
flags describe the synthetic scenario, not observed or state-specific gameplay. All actions are
read-only controls (`reobserve`, `safe_halt`, or `reconcile_action`); there are no play-card,
end-turn, purchase, co-op settlement, or state-specific mutation examples. Inventory and fixture
counts must not be reported as behavioral coverage of 131 game states.

## Intentionally open gates

The following remain `unverified` or `not performed`: inventory-materializer source reproduction;
direct v0.107.1 state discovery; exhaustive
state census confirmation; live screenshots and pixel signatures; mechanic parity; HTML/PDF
publication-quality rendering; HD Mermaid PNG rendering and visual inspection; expert-player
ratings; field/group ablations; autonomous trajectories; simulator/live differential parity;
co-op privacy/synchronization validation; and patch migration on a second executable.

These gaps are explicit because static source inspection, schema validity, a model response,
accepted action, reachable process, or recorded trajectory cannot prove game effect or runtime
compatibility. Promotion requires the exact build, platform, configuration, mods, hashes, evidence
capture, cleanup, and independent review described in the parent report.

## Required local checks

```sh
cargo run --locked --package sts2-expert-state-package -- docs/research/sts2-expert-state-package
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

The generated package is documentation/data material and does not authorize live mutation,
provider calls, game launch, save mutation, publication, or release tagging.
