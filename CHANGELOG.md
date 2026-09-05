# Changelog

All notable user-visible or operational changes to this project are documented here.

The project follows Semantic Versioning once versioned releases begin. Foundation work does not
claim a released harness version or runtime compatibility.

## Unreleased

### Added

- Complete the frozen Runtime-v1 consumer checksum inventory and golden messages; check both
  frozen runtime inventories in CI without changing existing wire schemas or manifests.

- A dated, evidence-labeled expert-state information-architecture research specification covering
  fair-play observation, the proposed atomic-state/action inventory, recovery, evaluation, and
  patch drift. It is explicitly not full-game or gameplay-proof evidence.

- A generated expert-state requirements package with 131 candidate states, typed observation/action/
  transition inventories, closed JSON schemas, synthetic fixture classes, per-state Markdown, and
  Mermaid sources. The package remains target-build validation material, not runtime support.

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

- The package is preparation-only: live providers, game access, gateway lease ownership, MCP framing,
  game rules, scoring, dataset export, and training integration remain outside this wave.

### Deprecated

- Nothing.

### Removed

- Nothing.

### Fixed

- Nothing.

### Security

- No provider, game, profile, save, credential, model, or dataset access was added.
