# Orchestration Prompt: Exo-Backed STS2 Autonomous Agent

Copy this document into the root orchestrator's task context when coordinating implementation of
the Exo-backed *Slay the Spire 2* agent. The hierarchy is intentional:

```text
orchestrator
  └── module agent       owns one architectural module
        └── component subagent  owns one cohesive component
              └── file specialist      owns one source, schema, test, or document file
```

The orchestrator may delegate work, but it remains responsible for dependency ordering, file
ownership, integration, evidence labels, safety, and the final release decision.

## Mission

Build and validate an autonomous STS2 gameplay system in which an LLM, hosted by Exo, chooses
semantic game actions through the existing fair-play control boundary. The finished system must be
able to launch a pinned STS2 instance, observe ordinary player-visible state, ask Exo for a bounded
decision, dispatch only an authoritative `LegalAction`, verify the resulting transition, and
continue through a complete run.

The system is an LLM agent, not a heuristic action selector. Deterministic code may calculate exact
facts, enumerate legal actions, reject unsafe choices, estimate outcomes, and recover from transport
failures. It must not silently choose a gameplay action when Exo or the configured model is
unavailable. A live unavailability must fail closed or enter an explicitly labeled operator/recovery
state.

The architecture is:

```text
STS2 Godot/C# host
  -> game-mod fair-play projection and authoritative LegalAction set
  -> gateway instance lifecycle, routing, leases, and fencing
  -> MCP semantic adapter
  -> harness episode coordinator
  -> Exo adapter and bounded LLM decision
  -> harness safety gate
  -> MCP -> gateway -> game-mod semantic execution
  -> transition barrier and independent postcondition verification
```

Exo is an external agent/orchestration dependency. Do not copy its implementation into any STS2
repository and do not make the game mod, gateway, MCP server, or protocol depend on Exo. Pin the
reviewed Exo revision before relying on a concrete API. If the Exo surface is not stable, use a
small process/HTTP/MCP adapter owned by the harness rather than an unpinned library dependency.

## Invocation contract

The root orchestrator receives:

```text
STS2_AGGREGATE_ROOT=/mnt/c/Users/timot/Documents/projects/sts2-project
STS2_INTEGRATION_ROOT=<isolated six-repository worktree root>
EXO_REPOSITORY=https://github.com/exoharness/exo
EXO_REVISION=<reviewed and pinned revision>
STS2_BUILD_ID=live-v0.107.1-build-23811903
STS2_BETA_BUILD_ID=beta-v0.111.0-build-24489008
TARGET_PLATFORM=Windows-x86_64
TARGET_LANGUAGE=en-US
TARGET_RESOLUTION=1920x1080
TARGET_UI_SCALE=100%
```

If a value is unavailable, mark the affected task `unverified`; never invent a hash, runtime trace,
API contract, or game rule. The public/live and beta builds are separate manifests. Beta behavior
must never silently update the live rules or compatibility database.

The orchestrator must first inspect the current status of all six repositories and create or select
isolated worktrees. The aggregate root is a non-Git coordination directory. Never reset, clean,
stash, broadly stage, overwrite, or delete unrelated user work. Never commit or push unless the
orchestrator's task explicitly authorizes it.

## Authority and non-negotiable invariants

Every agent, subagent, and specialist must obey these invariants:

1. The game mod and host own authoritative state, legality, mutation, host-thread affinity, and
   effect settlement.
2. The gateway owns instance launch, allocation, readiness, leases, routing, fencing, restart, and
   shutdown. The harness requests instances; it does not contact a game process directly.
3. The MCP server owns MCP framing and mapping. It remains a thin adapter.
4. `sts2-protocol` owns only named, language- and transport-neutral contracts with real producers and
   consumers. Do not create a generic `common` crate or duplicate protocol authority.
5. The harness owns coordination, provider ports, episodes, trajectories, replay, evaluation,
   datasets, and artifact lineage. Its core remains free of transports, game hosts, processes, and
   concrete provider implementations.
6. Exo receives a sanitized fair-play projection only. It may not read STS2 executables, PCK/DLL
   files, save files, host object graphs, raw memory, internal RNG state, unrevealed outcomes,
   private credentials, or unsanitized game logs.
7. A visible seed may be recorded as text. It must never be used to derive unrevealed random
   rewards, enemy moves, map content, draws, or event outcomes for the production policy.
8. The LLM may select from the current engine-generated semantic legal-action set. It may not emit
   screen coordinates, arbitrary input events, reflection paths, process commands, or raw HTTP game
   mutations.
9. No action is dispatched from a stale observation, during a blocking modal, while input is
   disabled, or after the action has left the current legal set.
10. No strategic retry occurs after an uncertain mutating action. Re-observe, reconcile, or enter a
    fail-closed recovery state first.
11. No model response, acknowledgement, reachable process, accepted action, score, or trajectory
    alone proves a completed semantic game effect.
12. Every claim is labeled `confirmed`, `source-derived`, `inferred`, `proposed`, `unverified`, or
    `unsupported`.

## Agent hierarchy and behavior

### Root orchestrator

The orchestrator is the only role that may change the execution graph. It must:

- create a manifest containing every module agent, component subagent, file specialist, owned file,
  dependency, status, and evidence level;
- assign exactly one owner to every file before parallel editing;
- prevent concurrent edits to the same file;
- maintain a dependency DAG and stop downstream agents when an upstream contract is unresolved;
- require subagents to report conflicts instead of editing another subagent's file;
- integrate in dependency order and resolve only documented conflicts;
- run the six-repository gates after each integration wave;
- keep static, build, component, host/runtime, and end-to-end evidence separate; and
- publish a final manifest stating what changed, what was tested, what remains unverified, and what
  was not committed, pushed, installed, launched, or released.

The orchestrator must not ask a specialist to make a cross-file “small fix” without assigning the
additional file or transferring ownership in the manifest.

### Module agent

A module agent owns one module below. It decomposes the module into component subagents, defines
their input/output contracts, sequences them, reviews their handoffs, and produces one module report.
It may edit only integration files explicitly assigned to it. It must not become a second authority
for another module.

### Component subagent

A component subagent owns one cohesive responsibility. It decomposes the responsibility into file
specialists, writes a short component contract before implementation, and reviews the specialists'
outputs for type, lifecycle, security, and evidence consistency. It must not directly edit an
unassigned specialist file.

### File specialist

A file specialist owns exactly one file unless the orchestrator assigns a generated file set as one
atomic unit. The specialist must:

- inspect the existing file and its tests before editing;
- preserve unrelated changes;
- implement only the component contract;
- keep production Rust within repository size and lint budgets;
- add or update focused tests when the file changes behavior;
- avoid hidden network, credentials, raw game access, and unbounded input;
- report assumptions and unresolved build-dependent facts; and
- return a bounded handoff with changed paths, tests, evidence, and remaining risks.

If a specialist discovers that a second file is required, it pauses and requests an ownership
transfer. It must not edit the second file opportunistically.

## Repository ownership map

| Module agent | Repository | Owns | Must not own |
|---|---|---|---|
| M1 Contract | `sts2-protocol` | neutral schemas, artifacts, conformance, protocol decisions | game rules, MCP framing, Exo SDK, host implementation |
| M2 Rules | `sts2-game-core` | observation-derived exact calculators, isolated belief-state simulation, domain validation | host objects, network, provider calls, authoritative future RNG |
| M3 Host bridge | `sts2-game-mod` | semantic observation, fair-play firewall, state/action authority, host-thread mutation | harness policy, Exo orchestration, gateway leases |
| M4 Gateway | `sts2-gateway` | game process supervisor, lifecycle, allocation, readiness, routing, fencing | card legality, LLM prompts, direct Exo calls |
| M5 MCP | `sts2-mcp-server` | semantic catalog, request mapping, response projection, MCP transport | game authority, model policy, Exo state |
| M6 Harness/Exo | `sts2-harness` | provider port, Exo adapter, agent session, decision records, bounded model execution | game process access, host assemblies, gateway internals |
| M7 Full-run loop | `sts2-harness` | state machine, episode loop, transition barriers, recovery, launch request orchestration | host legality, process ownership, model fallback actions |
| M8 Evaluation | `sts2-harness` plus assigned test/docs files | fair-play tests, replay, ablation, evidence, release gates | unreviewed performance claims, hidden-state evaluation |
| M9 Co-op | all six, additive only | player identity, votes, ally targets, synchronization, desync recovery | contamination of single-player contracts |
| M10 Patch/release | per-repository assigned files | build manifests, diffs, quarantine, CI, rollback, compatibility | silently promoting beta or inferred behavior |

## Dependency graph and spawn barriers

Use these barriers. Parallel work is allowed only within a barrier when file ownership is disjoint.

```text
B0 inventory and safety review
  -> B1 neutral contract and fair-play projection
  -> B2 host observation/legal actions + MCP/gateway mappings
  -> B3 Exo adapter + provider port + deterministic calculators
  -> B4 full-run episode loop + launch supervisor integration
  -> B5 live bounded Exo combat trace
  -> B6 noncombat and full-run progression
  -> B7 evaluation, co-op, patch automation, release promotion
```

At every barrier the orchestrator must run the relevant local tests and record the result. A later
barrier may use a stub or synthetic fixture, but it must label the result as offline evidence until
the real target build exercises it.

## Module assignments

The tables below assign one component subagent per cohesive responsibility and one file specialist
per path. A module agent may split a row further, but it must update the ownership manifest first.

### M1 — Neutral contract and provenance

**Objective:** Reuse `runtime-v3-gameplay-llm` unless a real cross-repository producer, consumer,
version, mapping, and conformance need requires a new neutral contract; never add an Exo-specific
protocol merely because the harness has an Exo adapter.

| Component subagent | File specialists | Acceptance |
|---|---|---|
| C1 decision envelope | `sts2-protocol/schemas/agent-decision-v1.schema.json`; `sts2-protocol/crates/protocol/src/agent_decision.rs`; `sts2-protocol/crates/protocol/src/lib.rs`; `sts2-protocol/crates/protocol/tests/agent_decision_conformance.rs`; `sts2-protocol/conformance/cases/agent-decision-v1.json` | May return “no new contract required”; otherwise bounded, versioned, neutral, and conformance-tested. |
| C2 artifacts and compatibility | `sts2-protocol/artifacts/agent-decision-v1/manifest.json`; `sts2-protocol/artifacts/agent-decision-v1/schema.json`; `sts2-protocol/artifacts/agent-decision-v1/SHA256SUMS`; `sts2-protocol/docs/decisions/0008-agent-decision-boundary.md`; `sts2-protocol/docs/COMPATIBILITY.md` | Artifacts name source, generator, license, inputs, digest, and regeneration command. |
| C3 research field classification | `sts2-harness/docs/research/sts2-expert-state-package/data/observations.json`; `sts2-harness/docs/research/sts2-expert-state-package/data/actions.json`; `sts2-harness/docs/research/sts2-expert-state-package/data/information-importance.csv`; `sts2-harness/docs/research/sts2-expert-state-package/data/claim-evidence-matrix.csv` | Every field has one access class and epistemic status; research data is not runtime authority. |

### M2 — Exact calculators and isolated simulator

**Objective:** Calculate from legitimate observations without becoming a host or future-RNG authority.
The simulator samples only explicit belief distributions.

| Component subagent | File specialists | Acceptance |
|---|---|---|
| C1 combat domain | `sts2-game-core/crates/core/src/combat.rs`; `sts2-game-core/crates/core/src/play_card.rs`; `sts2-game-core/crates/core/src/end_turn.rs`; `sts2-game-core/crates/core/src/state.rs`; `sts2-game-core/crates/core/src/validation.rs` | Preserve authoritative-boundary separation and reject malformed, duplicate, stale, or illegal inputs. |
| C2 calculators and belief state | `sts2-game-core/crates/core/src/calculators.rs`; `sts2-game-core/crates/core/src/probability.rs`; `sts2-game-core/crates/core/src/simulator.rs`; `sts2-game-core/crates/core/src/lib.rs` | Exact survival/lethal/resource outputs are distinct from labeled estimates; no seed-to-future-outcome shortcut. |
| C3 deterministic tests | `sts2-game-core/crates/core/tests/play_card.rs`; `sts2-game-core/crates/core/tests/end_turn.rs`; `sts2-game-core/crates/core/tests/domain_validation.rs`; `sts2-game-core/crates/core/tests/combat_calculators.rs`; `sts2-game-core/crates/core/tests/simulator_parity.rs` | Cover lethal witnesses, incoming damage, target domains, unknown randomness, and differential fixtures. |

### M3 — Authoritative game-mod bridge

**Objective:** Extend the host-side semantic bridge while retaining host-thread, state, legality,
mutation, and settlement authority. Managed loader code is the approved host exception.

| Component subagent | File specialists | Acceptance |
|---|---|---|
| C1 observation and state detection | `sts2-game-mod/experiments/managed-rust-interop/game-loader/RuntimeV3GameplayObservation.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/RuntimeV3GameplayCodec.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/RuntimeV3GameplayContract.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/RuntimeV3GameplaySupport.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/StateDetector.cs` | Distinguish actionable, transient, inspection, modal, unknown, and recovery states; unknown fails closed. |
| C2 fair-play projection | `sts2-game-mod/experiments/managed-rust-interop/game-loader/FairPlayProjection.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/PrivilegedFieldGuard.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/BuildManifest.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/ModEntry.cs` | Privileged fields are structurally absent; production-path injection tests prove the firewall. |
| C3 legal actions and settlement | `sts2-game-mod/experiments/managed-rust-interop/game-loader/LegalActionCatalog.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/PostconditionVerifier.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/RuntimeV3GameplayHost.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/LlmCombatPatch.cs` | Actions are typed, semantic, generation-bound, target-validated, and idempotency-aware; no policy coordinates. |
| C4 loader fixtures | `sts2-game-mod/experiments/managed-rust-interop/game-loader/GameLoaderProbe.csproj`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/mod_manifest.json`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/RuntimeV3GameplayFixtures.cs`; `sts2-game-mod/crates/game-mod/tests/runtime_v3_gameplay.rs`; `sts2-game-mod/crates/game-mod/tests/runtime_v2.rs`; `sts2-game-mod/crates/game-mod/tests/workshop.rs` | Existing runtime-v2 fixtures remain intact; host claims stay unverified until the pinned build runs. |

### M4 — Gateway process supervisor

**Objective:** Own launch, readiness, allocation, routing, fencing, restart, and shutdown without
moving game semantics or model policy into the gateway.

| Component subagent | File specialists | Acceptance |
|---|---|---|
| C1 lifecycle and supervisor | `sts2-gateway/crates/gateway/src/lifecycle.rs`; `sts2-gateway/crates/gateway/src/control.rs`; `sts2-gateway/crates/gateway/src/ports.rs`; `sts2-gateway/crates/gateway/src/process_supervisor.rs` | Explicit executable/config inputs, bounded readiness, disposable profiles, crash classification, cleanup, and sanitized records. |
| C2 runtime forwarding | `sts2-gateway/crates/gateway/src/bin/runtime_support/service.rs`; `sts2-gateway/crates/gateway/src/bin/runtime_support/runtime_v3_gameplay.rs`; `sts2-gateway/crates/gateway/src/bin/runtime_support/runtime_v3_gameplay_forwarder.rs`; `sts2-gateway/crates/gateway/src/bin/sts2-gateway-runtime.rs` | Runtime identity, lease, fencing, and forwarder behavior remain distinct and bounded. |
| C3 gateway tests | `sts2-gateway/crates/gateway/tests/control_plane.rs`; `sts2-gateway/crates/gateway/tests/runtime_v2.rs`; `sts2-gateway/crates/gateway/tests/process_supervisor.rs`; `sts2-gateway/crates/gateway/tests/support/mod.rs` | Deterministically test launch failure, timeout, crash/restart, stale lease, fencing, shutdown, cleanup, and isolation. |

### M5 — MCP semantic adapter

**Objective:** Map Exo-facing harness tools to versioned semantic operations while preserving MCP
thinness and gateway authority.

| Component subagent | File specialists | Acceptance |
|---|---|---|
| C1 catalog and action shape | `sts2-mcp-server/crates/mcp-server/src/catalog.rs`; `sts2-mcp-server/crates/mcp-server/src/catalog_runtime_v3_gameplay.rs`; `sts2-mcp-server/crates/mcp-server/src/projection_runtime_v3_gameplay_action_shape.rs`; `sts2-mcp-server/crates/mcp-server/src/projection_runtime_v3_gameplay_shape.rs` | Expose only bounded observe, legal-actions, dispatch, wait, reobserve, and recovery operations; no raw object or shell tool. |
| C2 mapping and transport | `sts2-mcp-server/crates/mcp-server/src/mapping_runtime_v3_gameplay.rs`; `sts2-mcp-server/crates/mcp-server/src/mapping_runtime_v3_gameplay_context.rs`; `sts2-mcp-server/crates/mcp-server/src/mapping_runtime_v3_gameplay_envelope.rs`; `sts2-mcp-server/crates/mcp-server/src/projection_runtime_v3_gameplay.rs`; `sts2-mcp-server/crates/mcp-server/src/server.rs`; `sts2-mcp-server/crates/mcp-server/src/transport.rs` | Preserve profile identity, bounds, correlation, generation, redaction, and semantic parameter mapping. |
| C3 mapping tests | `sts2-mcp-server/crates/mcp-server/tests/runtime_v3_gameplay_mapping.rs`; `sts2-mcp-server/crates/mcp-server/tests/runtime_v2_mapping.rs`; `sts2-mcp-server/crates/mcp-server/tests/seam.rs`; `sts2-mcp-server/crates/mcp-server/tests/runtime_v3_exo_tools.rs` | Prove stale rejection, unknown-state handling, redaction, and one-to-one action mapping. |

### M6 — Harness provider and Exo adapter

**Objective:** Put Exo behind the provider port and refactor the current Ollama-compatible lane so
both providers use the same typed request, response, identity, and safety gate.

| Component subagent | File specialists | Acceptance |
|---|---|---|
| C1 provider contract | `sts2-harness/crates/harness/src/provider.rs`; `sts2-harness/crates/harness/src/error.rs`; `sts2-harness/crates/harness/src/identity.rs`; `sts2-harness/crates/harness/src/lib.rs` | Preserve bounded I/O, model identity, idempotency, cancellation, retry rules, credentials, and typed errors. |
| C2 Exo client/session/sandbox | `sts2-harness/crates/harness/src/exo/mod.rs`; `sts2-harness/crates/harness/src/exo/client.rs`; `sts2-harness/crates/harness/src/exo/protocol.rs`; `sts2-harness/crates/harness/src/exo/session.rs`; `sts2-harness/crates/harness/src/exo/sandbox.rs` | Pin Exo, bound I/O, redact credentials, classify unavailable responses, and expose only sanitized observations/records. |
| C3 decision parser and binaries | `sts2-harness/crates/harness/src/exo/decision.rs`; `sts2-harness/crates/harness/src/bin/sts2-harness-llm-combat.rs`; `sts2-harness/crates/harness/src/bin/sts2-harness-exo.rs`; `sts2-harness/experiments/exo-agent/README.md`; `sts2-harness/experiments/exo-agent/config.example.toml` | Strict structured response with concise rationale; malformed/illegal output rejected; no chain-of-thought or action fallback. |
| C4 provider tests | `sts2-harness/crates/harness/tests/provider.rs`; `sts2-harness/crates/harness/tests/exo_adapter.rs`; `sts2-harness/crates/harness/tests/fair_play_firewall.rs`; `sts2-harness/crates/harness/tests/provider_redaction.rs` | Fake Exo tests cover timeout, malformed/oversized output, duplicates, cancellation, unavailability, injection, and legality. |

### M7 — Full-run episode and launch orchestration

**Objective:** Replace combat-only control with setup, map, combat, rewards, shop, events, rest, deck
selection, victory/defeat, save/quit, and recovery. The loop requests launch through the gateway.

| Component subagent | File specialists | Acceptance |
|---|---|---|
| C1 episode state machine | `sts2-harness/crates/harness/src/episode/mod.rs`; `sts2-harness/crates/harness/src/episode/state_machine.rs`; `sts2-harness/crates/harness/src/episode/observation.rs`; `sts2-harness/crates/harness/src/episode/legal_actions.rs`; `sts2-harness/crates/harness/src/episode/transition.rs` | Each atomic state declares required fields, legal actions, actionability, successors, timeouts, and recovery. |
| C2 barriers and recovery | `sts2-harness/crates/harness/src/episode/stability_barrier.rs`; `sts2-harness/crates/harness/src/episode/postconditions.rs`; `sts2-harness/crates/harness/src/episode/recovery.rs`; `sts2-harness/crates/harness/src/episode/idempotency.rs` | Use semantic/stable-snapshot evidence, never global sleep; uncertain mutation stops dispatch and reconciles. |
| C3 policy routing | `sts2-harness/crates/harness/src/episode/policy_router.rs`; `sts2-harness/crates/harness/src/episode/run_setup.rs`; `sts2-harness/crates/harness/src/episode/noncombat.rs`; `sts2-harness/crates/harness/src/episode/shutdown.rs`; `sts2-harness/crates/harness/tests/full_run.rs` | Every gameplay choice reaches Exo; deterministic code only hard-eliminates or calculates. |

### M8 — Verification, memory, replay, and evaluation

**Objective:** Prove safety and measure quality without turning one run into a broad performance claim.

| Component subagent | File specialists | Acceptance |
|---|---|---|
| C1 typed records and memory | `sts2-harness/crates/harness/src/records.rs`; `sts2-harness/crates/harness/src/decision_records.rs`; `sts2-harness/crates/harness/src/memory.rs`; `sts2-harness/crates/harness/src/artifact.rs` | Bounded typed memory distinguishes observations, requests, acceptance, settlement, recovery, estimates, and unavailable evidence. |
| C2 replay and evaluation | `sts2-harness/crates/harness/src/replay.rs`; `sts2-harness/crates/harness/src/evaluation.rs`; `sts2-harness/crates/harness/tests/replay.rs`; `sts2-harness/crates/harness/tests/evaluation.rs` | Measure legality, staleness, verification, recovery, regret, calibration, resource use, progression, and completion. |
| C3 visual watchdog | `sts2-harness/experiments/cv-watchdog/README.md`; `sts2-harness/experiments/cv-watchdog/config.example.toml`; `sts2-harness/experiments/cv-watchdog/fixtures/README.md`; `sts2-harness/docs/evidence/cv-watchdog-validation.md` | CV independently reports disagreement; it never becomes a hidden-state or alternate-authority source. |
| C4 evidence and ablation | `sts2-harness/docs/evidence/exo-live-combat-<date>.md`; `sts2-harness/docs/evidence/exo-full-run-<date>.md`; `sts2-harness/experiments/ablation/README.md`; `sts2-harness/experiments/ablation/config.example.toml` | Run remove, mask, delay, stale, corruption, and confidence-degradation studies with build lineage. |

### M9 — Cooperative-mode extension

**Objective:** Add co-op only after single-player stability, using additive peer/vote/ally fields and
suspending autonomous mutation during disagreement.

| Component subagent | File specialists | Acceptance |
|---|---|---|
| C1 neutral co-op contract | `sts2-protocol/schemas/coop-gameplay-v1.schema.json`; `sts2-protocol/crates/protocol/src/coop_gameplay.rs`; `sts2-protocol/crates/protocol/tests/coop_gameplay_conformance.rs` | Separate local action, shared vote, shared effect, ally target, and synchronization types. |
| C2 host/gateway synchronization | `sts2-game-mod/experiments/managed-rust-interop/game-loader/CoopProjection.cs`; `sts2-game-mod/experiments/managed-rust-interop/game-loader/CoopSynchronization.cs`; `sts2-gateway/crates/gateway/src/coop_session.rs`; `sts2-gateway/crates/gateway/tests/coop_session.rs` | Player identity and desync behavior are explicit, bounded, and fail closed. |
| C3 MCP/harness coordination | `sts2-mcp-server/crates/mcp-server/src/mapping_coop_gameplay.rs`; `sts2-harness/crates/harness/src/episode/coop.rs`; `sts2-harness/crates/harness/tests/coop.rs` | Disconnect, peer disagreement, and unknown shared effects suspend mutation. |

### M10 — Patch drift and release promotion

**Objective:** Quarantine every build change and promote it only after reproducible validation.

| Component subagent | File specialists | Acceptance |
|---|---|---|
| C1 build/patch manifests | `sts2-harness/docs/research/sts2-expert-state-package/data/build-manifest.json`; `sts2-harness/patch-manifest.schema.json`; `sts2-harness/tools/patch-diff/src/main.rs`; `sts2-harness/tools/patch-diff/README.md` | Record build/data/UI/action/schema diffs and quarantine status. |
| C2 CI and policy | `.github/workflows/ci.yml` in each of `sts2-protocol`, `sts2-game-core`, `sts2-game-mod`, `sts2-gateway`, `sts2-mcp-server`, and `sts2-harness` | Add focused checks without weakening existing policy or deleting unrelated workflows. |
| C3 release evidence | `sts2-harness/docs/COMPATIBILITY.md`; `sts2-harness/docs/TESTING.md`; `sts2-harness/RELEASING.md`; `sts2-harness/docs/evidence/release-gate-<build>.md` | Require hashes, schema/action diffs, leak tests, deterministic tests, host/full-run/co-op evidence, cleanup, and rollback. |

## Exo tool and prompt contract

The Exo adapter must provide a narrow tool surface. Names may be mapped to the chosen Exo API, but
the semantic meaning cannot change:

```text
sts2.observe
  returns current fair-play GameObservation and freshness/source metadata

sts2.legal_actions
  returns the host-generated legal action set for the same observation generation

sts2.dispatch_action
  accepts exactly one typed LegalAction plus generation/idempotency identity

sts2.wait_for_transition
  waits for a semantic successor, same-state mutation, or bounded timeout

sts2.reobserve
  obtains a fresh ordinary observation after a contradiction or stale result

sts2.recover
  performs only an explicitly safe recovery operation; never a strategic action
```

The decision prompt sent through Exo must contain:

- the current state ID and transition/generation identity;
- fair-play visible, on-demand, historical, and derived-exact fields;
- explicitly labeled estimates and uncertainty;
- the complete current legal-action set or a bounded reference to it;
- objective and hard constraints;
- output schema and maximum response size; and
- a requirement for one selected action or an explicit `WAIT`, `REOBSERVE`, or `RECOVERY` result.

It must not contain hidden fields, raw host dumps, future RNG state, unrevealed random outcomes,
private prompts, credentials, or unrestricted tool instructions. The model response must not be
stored verbatim in ordinary trajectory artifacts.

## File-specialist handoff format

Every specialist returns exactly one bounded handoff object or its Markdown equivalent:

```json
{
  "specialist_id": "M6.C2.S03",
  "role": "file-specialist",
  "owned_file": "crates/harness/src/exo/session.rs",
  "status": "complete|blocked|no-change|unverified",
  "changed_paths": [],
  "contract": "one sentence describing the file responsibility",
  "assumptions": [],
  "evidence": [
    {
      "kind": "unit|component|host|end_to_end|source",
      "status": "confirmed|source-derived|inferred|proposed|unverified|unsupported",
      "command_or_artifact": "..."
    }
  ],
  "tests": [
    {"command": "...", "result": "pass|fail|skipped", "reason": "..."}
  ],
  "security_review": {
    "privileged_fields_exposed": false,
    "credentials_persisted": false,
    "unbounded_input": false,
    "raw_game_access": false
  },
  "remaining_risks": [],
  "needs_from_other_specialist": []
}
```

A `complete` status means the file contract is implemented and focused tests pass; it does not mean
the six-repository system or target game is integrated. A live claim requires a separate runtime
handoff with exact build/configuration/evidence details.

## Component and module handoff requirements

Before the next barrier, each component subagent provides:

1. its list of accepted specialist handoffs;
2. the final component API/schema and ownership statement;
3. its file-conflict and dependency review;
4. focused test results and unverified runtime gates; and
5. a statement that no privileged data path or raw-action path was introduced.

Each module agent provides:

1. the component reports;
2. the module dependency and migration note;
3. changed contract and compatibility identifiers;
4. a cross-file test plan;
5. evidence classification; and
6. a recommendation to proceed, quarantine, or stop.

## Required test sequence

The orchestrator runs tests in this order and records exact commands/results:

1. Per-file specialist tests and static validation.
2. Per-module policy, formatting, lint, unit, and component tests.
3. Cross-repository schema/artifact/conformance checks.
4. Fake Exo process and fake MCP/gateway integration.
5. Privileged-field injection and redaction tests.
6. Stale observation, delayed transition, double-dispatch, unknown-state, crash, and desync fault
   injection.
7. Exact pinned-build managed/native package build and hash manifest, when the licensed installation
   is available.
8. Bounded Exo live combat trace with no heuristic action fallback.
9. Launch/setup/map/reward/rest/shop/event progression trace.
10. Full-run trace with defeat and victory outcomes reported separately.
11. Co-op trace for two, three, and four instances if M9 is enabled.
12. Clean-install/replay/rollback smoke and final policy gates.

For Rust repositories, the default local gates are:

```bash
cargo fmt --all --check
cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings
cargo test --locked --offline --workspace
cargo run --locked --offline --package repo-policy -- --strict
git diff --check
```

If a command requires the game, Exo, a provider, a licensed installation, or a network service,
label it `unverified` when that dependency is unavailable. Never convert a skipped runtime check
into a pass.

## Definition of done

The orchestration is complete only when all of the following are true:

- Exo is pinned and connected through a reviewed harness adapter.
- The LLM, not a gameplay heuristic fallback, supplies gameplay decisions.
- The production Exo workspace contains only sanitized fair-play data.
- Game launch and lifecycle are owned by the gateway and observable by the harness.
- Every dispatched action is a current, host-generated `LegalAction`.
- Every mutating action has a verified postcondition or enters bounded recovery.
- Unknown states, provider failure, stale observations, double actions, and desynchronization fail
  closed.
- Setup, combat, map, reward, shop, event, rest, selection, victory, defeat, and recovery surfaces
  have target-build validation status.
- No privileged-information leak is detected by static or runtime tests.
- Deterministic and live evidence are reported separately with exact build/configuration lineage.
- The six repositories pass their required local gates without unrelated changes being staged.
- Any unverified acceptance criterion is explicitly listed; no inferred STS2 rule is presented as
  observed fact.

The final invariant is:

```text
stable state
  AND fair-play observation
  AND current LegalAction
  AND Exo decision
  AND verified transition
  AND no PRIVILEGED data
```

If any term is false, the agent must not continue autonomous gameplay.
