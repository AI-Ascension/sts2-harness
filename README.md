<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/AI-Ascension/.github/main/profile/assets/banner-dark.svg">
  <img alt="AI-Ascension — Inspect how AI requests to a game get fenced, one Rust contract at a time. Bounded runtime host trace confirmed. Deterministic tests: confirmed." src="https://raw.githubusercontent.com/AI-Ascension/.github/main/profile/assets/banner-light.svg" width="100%">
</picture>

# sts2-harness

> **AI-Ascension · flagship · tier 4: experiment coordinator** — Experiment coordinator for AI runs: episodes, a pluggable model-provider interface, replay of recorded records, and artifact lineage.
>
> **Status:** deterministic tests and one bounded `runtime-v1` host trace `confirmed` for STS2 v0.107.1 on Windows x86-64 · providers, gameplay mutation, and broader compatibility `unverified`.
> **Proof:** [45-second browser replay](https://ai-ascension.github.io/proof.html) · [Evidence ledger](https://ai-ascension.github.io/evidence.html) · [This repository on the map](https://ai-ascension.github.io/repositories.html#sts2-harness)
> **Start here:** the harness is the flagship entry point for the organization; the public proof currently lives in [sts2-gateway](https://github.com/AI-Ascension/sts2-gateway) because that is where the first fenced boundary is tested.
> **Owner:** The harness maintainers own the experiment control plane and its records: coordination, provider ports, runs and episodes, trajectories, replay, and artifact lineage.
> **Contribute:** [Organization guide](https://github.com/AI-Ascension/.github/blob/main/CONTRIBUTING.md) · [First tasks](https://ai-ascension.github.io/contributing.html)
>
> AI-Ascension is an independent project. It is not affiliated with or endorsed by Mega Crit or Valve and grants no rights to game files, assets, or marks.

Status: Wave 2 codebase initialization plus a bounded runtime coordinator. The target-owned harness
package contains pure coordinator ports and deterministic fake-boundary tests; an authorized
`runtime-v1` trace now confirms the coordinator-to-host path for one exact disposable STS2 profile.
This target is distinct from any legacy or reference checkout and contains no game files, model
weights, datasets, provider credentials, or generated product artifacts.

The current research baseline is the self-contained
[`STS2 Expert-State Information Architecture`](docs/research/STS2_EXPERT_STATE_INFORMATION_ARCHITECTURE_2026-09-03.md).
It is a proposed harness-facing research specification: it separates fair-play observation from
privileged data, records the initial 144-state inventory as requiring target-build validation, and
does not turn the bounded host probe into full gameplay or state-discovery evidence.

The companion [`generated expert-state package`](docs/research/sts2-expert-state-package/README.md)
contains the 131-state requirements baseline, typed inventories, JSON schemas, synthetic fixtures,
per-state Markdown requirements, and Mermaid source diagrams. It is generated research material,
not a game adapter or target-build certification.

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
This target has source/documentation evidence, deterministic offline fake-test evidence, a dated
component trace against a synthetic downstream, and a dated exact-host runtime trace. Provider
execution, gameplay-rule mutation, replay fidelity against a game, scoring validity, training
outcomes, and compatibility beyond the recorded host remain unverified. See
[`docs/evidence/runtime-v1-host-integration-20260902.md`](docs/evidence/runtime-v1-host-integration-20260902.md).

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

## Runtime slice coordinator

The standalone `sts2-harness-runtime` binary is the explicit coordinator for the first
`runtime-v1` trace. It allocates one configured gateway lease, starts the MCP process, performs
initialize/list/state/action/stale-action/fresh-state calls, checks the effect witness and stable
rejection, closes MCP stdin, and releases the lease. It has no direct game or mod access.

The process uses configured bearer tokens and separate instance, gateway session, MCP session, lease,
and epoch identities. Runtime-v2 process records additionally emit a bounded
`lineage` object containing `run_id`, `episode_id`, `trajectory_id`, and `artifact_id`, plus the
MCP request-ID sequence and downstream correlation IDs for each observed response. Callers may
override the redacted record identities with `STS2_RUN_ID`, `STS2_EPISODE_ID`,
`STS2_TRAJECTORY_ID`, and `STS2_ARTIFACT_ID`; the harness rejects empty, unsafe, oversized, or
colliding lineage values. The synthetic component run and the authorized exact-host run are
recorded separately. The latter confirms the managed host callback and bounded STS2 effect for the
safe probe; gameplay mutation and broader compatibility remain `unverified`.

The same binary has an opt-in `runtime-v3-gameplay` profile. Set `STS2_RUNTIME_PROFILE` to that
value and provide the exact reviewed `STS2_EXO_REVISION`, direct `STS2_EXO_BRIDGE_BINARY`, and
`STS2_OBJECTIVE` inputs. The profile requires the six semantic MCP tools, keeps host payloads at the
MCP boundary, and fails closed when the gateway, MCP process, Exo bridge, or target runtime is
missing. For this profile `STS2_MCP_SESSION_ID` defaults to and must equal `STS2_SESSION_ID` so
gateway and MCP protocol identity remain fenced to one session. Live Exo and target-game behavior
remain `unverified` until a separate runtime handoff.

## Runtime-v2 deterministic fake lane

The separate `sts2-harness-runtime-v2-fake` binary consumes the copied `runtime-v2` release-like
artifact, verifies its source/package bytes and checksums, and runs one in-memory instance through
`requested -> accepted -> unknown -> reconciled -> settled`, a fresh generation `N+1` observation,
duplicate replay without a second mutation, and stale-epoch rejection. It does not change the
Runtime-v1 coordinator or contact a live host, game, provider, model, profile, save, or network.
Live host settlement, gameplay mutation, provider/model execution, and Runtime-v2 compatibility
remain `unverified`; see
[`docs/evidence/runtime-v2-fake-20260902.md`](docs/evidence/runtime-v2-fake-20260902.md).
