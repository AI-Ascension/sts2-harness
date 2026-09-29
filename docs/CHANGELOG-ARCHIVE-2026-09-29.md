# Changelog archive: 2026-09-29

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link
paths, which are corrected so they resolve from this directory; this file is a verbatim record,
not a supported release or a second normative changelog.

### Archived from CHANGELOG.md

- **Make the served capture surface configured and fail-closed.** Which sink the served composition
  attaches and what it retains is now an owner decision recorded in
  [ADR 0070](decisions/0070-served-capture-configuration-and-retention.md): an unset surface
  keeps the merged in-memory recording ring, `metadata` and `off` are selectable, and every
  contradictory or out-of-range `STS2_WORKFLOW_CAPTURE_*` value is refused at startup rather than
  silently downgraded. Restart-durable capture bytes and the unrecorded Ollama `HttpBody` boundary
  remain accountable residuals (#145). Compatibility: no change to the served default behaviour.
  Refs #398.

- **Admit alternative gameplay forks from verified seeded replay prefixes.** A new
  `benchmark_manifest::prefix_fork` module fixes the effect-free fork-admission contract behind
  #117: an exact seed/profile/build/compatibility binding, a settled nonterminal boundary with
  complete receipts and one resolved legal action, a zero-provider-call replay, bounded sibling
  forks with distinct identities, and a forward-only replay-to-child handoff that reconciles a lost
  target. Source-only ([ADR 0069](decisions/0069-prefix-fork-admission.md)). Refs #117.

- **Orchestrate isolated cold-launch benchmark trials from one pristine baseline.** A new
  `benchmark_manifest::cold_launch` module fixes the per-trial isolation contract behind issue #122:
  an immutable baseline binding the artifact digest, launch profile and closed telemetry exclusions;
  an exclusive, bounded destination lease; an opaque gateway-attested process birth with its own
  instance generation, since a PID can be reused; a readiness proof bound to that birth; a recorded
  stage machine that reconciles a lost reply without adopting another trial's state and quarantines
  an uncertain destination; and machine-readable cold-start evidence whose cleanup failure is
  distinct from the gameplay outcome. Source-only: native process evidence and the real child-process
  lane stay gated by sts2-game-mod#79
  ([ADR 0068](decisions/0068-cold-launch-trial-isolation.md)). Refs #122.

- **Bind a benchmark rerun admission to the exact declaration it compared equal.** `RerunAdmission`
  now owns the admitted `Manifest`, reachable only through `RerunAdmission::declaration()`, so a
  `RerunAllocationSeam` cannot allocate for a declaration other than the one whose controlled inputs
  compared equal. Source-only contract tightening for #121; the equal path is unchanged.
  Refs #121.
