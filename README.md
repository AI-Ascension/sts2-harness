<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/AI-Ascension/.github/main/profile/assets/banner-dark.svg">
  <img alt="AI-Ascension — Inspect how AI requests to a game get fenced, one Rust contract at a time. Runtime: unverified. Deterministic tests: confirmed." src="https://raw.githubusercontent.com/AI-Ascension/.github/main/profile/assets/banner-light.svg" width="100%">
</picture>

# sts2-harness

> **AI-Ascension · flagship · tier 4: experiment coordinator** — Experiment coordinator for AI runs: episodes, a pluggable model-provider interface, replay of recorded records, and artifact lineage.
>
> **Status:** deterministic in-memory tests `confirmed` at the pinned commit · runtime, host, and game compatibility `unverified` · nothing is live.
> **Proof:** [45-second browser replay](https://ai-ascension.github.io/proof.html) · [Evidence ledger](https://ai-ascension.github.io/evidence.html) · [This repository on the map](https://ai-ascension.github.io/repositories.html#sts2-harness)
> **Start here:** the harness is the flagship entry point for the organization; the public proof currently lives in [sts2-gateway](https://github.com/AI-Ascension/sts2-gateway) because that is where the first fenced boundary is tested.
> **Owner:** The harness maintainers own the experiment control plane and its records: coordination, provider ports, runs and episodes, trajectories, replay, and artifact lineage.
> **Contribute:** [Organization guide](https://github.com/AI-Ascension/.github/blob/main/CONTRIBUTING.md) · [First tasks](https://ai-ascension.github.io/contributing.html)
>
> AI-Ascension is an independent project. It is not affiliated with or endorsed by Mega Crit or Valve and grants no rights to game files, assets, or marks.

Status: Wave 2 codebase initialization. The target-owned harness package contains pure coordinator
ports and deterministic fake-boundary tests; live product behavior remains unverified. This
target is distinct from any legacy or reference checkout and contains no game files, model weights,
datasets, provider credentials, or generated product artifacts.

## Owner and consumers

The target owner is the harness maintainers. The harness owns the experiment control plane and the
records produced by it: multi-instance coordination, model/provider ports, runs and episodes,
observations and actions, trajectories, deterministic replay, scoring/evaluation, dataset preparation,
and artifact lineage.

It consumes the MCP server and gateway through declared interfaces. The MCP server is the thin adapter
for its protocol; the gateway is the lifecycle/routing control plane. Provider implementations enter
through an explicit provider port. Operators, replay tools, evaluators, training jobs, and artifact
stores consume harness-owned records through versioned interfaces. Runtime communication is not a
permission to import another repository's implementation crate.

## Boundary and non-goals

The harness has no direct game access. It does not load host assemblies, call game objects, own game
rules or legal actions, launch or route game processes, manage leases, implement MCP framing, or
replace gateway authority. The game mod/host remains authoritative for state and mutation at the host
boundary. Core coordination policy must remain free of transports, hosts, processes, and concrete
providers.

Wave 2 adds one non-empty target-owned Rust package at `crates/harness`. It owns run and episode
identity, route requests, provider-neutral model execution, append-only record seams, deterministic
replay evaluation, artifact metadata/lineage, and bounded shutdown orchestration. Its tests use only
deterministic in-memory fakes. It does not add a provider call, game launch, gateway lease manager,
MCP framing, game rule, scorer, dataset exporter, or training integration.

## Runtime topology

```text
model/provider -- declared provider port --> harness
harness -- MCP client --> MCP server -- control/interaction --> gateway
gateway -- owned allocation/lease/routing --> isolated game-mod instances --> game host
harness -- versioned records/artifacts --> approved artifact store or offline consumer
```

The arrows are runtime contracts only. The compile-time graph is adapter and port based; it must not
depend on game-host or game-mod implementation code. The harness cannot bypass MCP or gateway to
contact a game process.

## Contract and protocol scope

`sts2-protocol` is accepted as the sixth implementation target. It owns only genuinely shared,
language- and transport-neutral contract artifacts: namespace-qualified correlation/lineage metadata,
independent version/profile descriptors, selected lifecycle/deadline/error envelope metadata, schema
manifests, and implementation-neutral conformance vectors. It is not a runtime service and does not
own game semantics, HTTP/MCP framing, gateway leases, provider behavior, storage, or a generic
`common` implementation crate.

Named prospective consumers are `sts2-game-core`, `sts2-gateway`, `sts2-mcp-server`, and
`sts2-harness`; each consumes only the protocol profiles mapped to its boundary. `sts2-game-mod`
remains authoritative for host access and mutation and is not a current protocol consumer unless a
later decision accepts a specific neutral artifact. Protocol releases, schema/profile versions, and
consumer versions remain independent. Consumers record the protocol revision/profile and schema
digest in compatibility and artifact lineage records. The accepted decision, including conformance
requirements, is recorded in
[`docs/decisions/0002-sixth-target-protocol-decision.md`](docs/decisions/0002-sixth-target-protocol-decision.md).

The minimal deterministic POC consumes the copied release-like artifact at
`protocol-artifact/poc-v1/`, whose manifest records `sts2-protocol/poc-v1`, the source schema,
generator, and schema digest. Its fake vertical slice is exactly:
`harness -> MCP -> gateway -> game-mod -> game-core`. The runner emits one canonical trace event
per boundary for a state read, an accepted `use_budget` action, and a rejected zero-unit action.
This is offline source/test evidence only; it does not establish live transport, process, host,
game, provider, or compatibility behavior. See [`MINIMAL_POC_REPORT.md`](MINIMAL_POC_REPORT.md).

Harness-owned contracts keep these namespaces distinct: `instance_id`, `session_id`, `run_id`,
`episode_id`, `trajectory_id`, `request_id`, `action_id`, `trace_id`, `model_execution_id`, and
`artifact_id`. Records bind only the independent versions that affect them, including harness,
trajectory/schema, scoring, training/dataset, provider profile, MCP, gateway, and game/mod versions.
An acknowledgement, model response, accepted action, or trajectory does not by itself prove effect,
semantic correctness, reproducibility, or runtime compatibility.

## Evidence and provenance

Claims use `confirmed`, `source-derived`, `inferred`, `proposed`, `unverified`, or `unsupported`.
This target has source/documentation evidence and deterministic offline fake-test evidence only.
Gateway/MCP interaction, provider execution, game state/action behavior, replay fidelity against a
game, scoring validity, training outcomes, and packaged runtime compatibility remain unverified until
controlled tests exist.

Imported or generated records must carry origin, license, generator, input identity, and digest. Do not
copy or transliterate reference implementation source. Do not retain credentials, private prompts or
model output, valued saves, personal paths, proprietary host bytes, or unsanitized multiplayer/game
text.

## Layout and local validation

Responsibility directories are `crates/harness`, `experiments`, `schemas`, `conformance`, `docs`,
`tests`, and `tools`. The repository foundation and its rules are described in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), [`docs/PRODUCT.md`](docs/PRODUCT.md), and
[`docs/REPOSITORY_LAYOUT.md`](docs/REPOSITORY_LAYOUT.md). The staged planning prompt remains an input
for future specification work; it is not implementation evidence.

The local read-only validation entrypoint is:

```bash
cargo run --locked --package repo-policy -- --strict
cargo metadata --locked --no-deps --format-version 1
```

For Rust changes, also run `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, and
`cargo test --workspace --all-targets --all-features --locked`. Missing runtime dependencies are
reported as unverified rather than converted into a pass.
