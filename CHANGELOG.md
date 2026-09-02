# Changelog

All notable user-visible or operational changes to this project are documented here.

The project follows Semantic Versioning once versioned releases begin. Foundation work does not
claim a released harness version or runtime compatibility.

## Unreleased

### Added

- Repository governance, policy-as-code, workflow, licensing, security, and release foundations.
- Harness-specific ownership, dependency, protocol-repository, compatibility, and provenance decisions.
- Documentation for multi-instance coordination, model/provider ports, episodes, trajectories,
  replay, scoring, evaluation, and artifact lineage.
- A target-owned Rust harness package with explicit routing, provider, record, replay, artifact, and
  shutdown ports plus deterministic fake-boundary tests.

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
