# Changelog

All notable user-visible or operational changes to this project are documented here.

The project follows Semantic Versioning once versioned releases begin. Foundation work does not
claim a released harness version or runtime compatibility.

## Unreleased

- Release router bindings rejected for mismatched run or episode identity and surface cleanup
  failures through the routing error boundary.
- Split the independent Runtime-v2 coordinator and process trace from PR #7; the bounded
  Runtime-v3 card-play probe remains outside this change. Frozen artifact bytes are unchanged.

### Added

- Complete the frozen Runtime-v1 consumer checksum inventory and golden messages; check both
  frozen runtime inventories in CI without changing existing wire schemas or manifests.

- A dated, evidence-labeled expert-state information-architecture research specification covering
  fair-play observation, the proposed atomic-state/action inventory, recovery, evaluation, and
  patch drift. It is explicitly not full-game or gameplay-proof evidence.

- A generated expert-state requirements package with 131 candidate states, typed observation/action/
  transition inventories, closed JSON schemas, synthetic fixture classes, per-state Markdown, and
  Mermaid sources. The package remains target-build validation material, not runtime support.

- A bounded Runtime-v2 multi-instance coordinator seam with four-lane registration, explicit
  identity isolation, fair serial dispatch, global/per-instance backpressure, queued cancellation,
  active-work reconciliation reporting, and sanitized snapshots. This is component evidence only.
- Propagated the independently configured Runtime-v2 MCP session through gateway allocation,
  spawned MCP configuration, request correlation, and lease release. The gateway session remains
  the frozen protocol-envelope identity; gateway and MCP session values must be distinct.

### Safety corrections

- Require a `released` status after runtime lease cleanup; a successful HTTP exchange alone
  no longer counts as confirmed release.

- Retain unknown operations in their serial instance lane until explicit reconciliation.
- Bound MCP and loopback gateway exchanges end to end, reap owned MCP children, and reject
  mismatched responses without printing downstream payloads or inheriting unrelated credentials.
- Validate exact Runtime-v2 response contracts and retain only numeric legacy gameplay trace fields.

- Require recovered transitions to match the complete dispatched action, and reconnect failed MCP
  transports only for bounded recovery reads while retaining operation identity. Attempt fenced
  lease cleanup when an allocation response is lost or invalid.
- Split Exo request validation, fair-play schema rules, decision replay, and evaluation report
  projection into cohesive modules within ordinary policy budgets; remove handwritten exemptions.

- The bounded Runtime-v3 episode state machine, semantic action ledger, transition barrier and
  recovery ports, and strict Exo fair-play decision adapter.
- A bounded complete-run coordinator that routes every declared playable surface through the
  current host legal-action catalog and independently verifies transition settlement.
- An operator-owned direct Exo process transport with bounded stdin/stdout, timeout, environment
  allowlisting, and fail-closed shutdown behavior.
- An offline Exo configuration example and explicit `unverified` live-connectivity status.
- Full-run routing coverage for setup, map, combat, reward, shop, event, rest, selection, and
  separate victory/defeat terminal observations.
- Bounded typed decision records, memory, replay/evaluation metrics, cooperative synchronization
  gates, and quarantined M10 build/patch manifest preparation.
- The bounded `sts2-harness-runtime` coordinator, `runtime-v1` artifact copy, real MCP/gateway
  process path, stale-generation oracle, sanitized trace, and component evidence record.

- A dated authorized-host integration record confirming the complete bounded coordinator-to-STS2
  runtime probe, visible effect witness, stale-generation rejection, and reversible cleanup.

- Repository governance, policy-as-code, workflow, licensing, security, and release foundations.
- Harness-specific ownership, dependency, protocol-repository, compatibility, and provenance decisions.
- Documentation for multi-instance coordination, model/provider ports, episodes, trajectories,
  replay, scoring, evaluation, and artifact lineage.
- A target-owned Rust harness package with explicit routing, provider, record, replay, artifact, and
  shutdown ports plus deterministic fake-boundary tests.
- A copied release-like `sts2-protocol/poc-v1` artifact, deterministic five-boundary fake runner, and
  [`MINIMAL_POC_REPORT.md`](MINIMAL_POC_REPORT.md) with the canonical 15-event trace.

### Changed

- Standardized historical POC and Runtime-v2 fake evidence labels to `confirmed`, retaining
  their deterministic-fake scope, original dates, trace bytes, digests, and unverified live lanes.

- Attempt fenced allocation cleanup when the runtime coordinator cannot accept an allocation
  response, preserving the configured trace fence and reporting release failures explicitly.
  Deterministic fake-boundary coverage does not establish live cleanup behavior.

- The package is preparation-only: live providers, game access, gateway lease ownership, MCP framing,
  game rules, scoring, dataset export, and training integration remain outside this wave.
- Exo revisions are now required to be exact non-zero lowercase commit hashes; the checked-in
  example uses the reviewed public audit revision and does not claim live connectivity.

### Deprecated

- Nothing.

### Removed

- Nothing.

### Fixed

- Nothing.

### Security

- No provider, game, profile, save, credential, model, or dataset access was added.
