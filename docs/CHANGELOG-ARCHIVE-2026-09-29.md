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

- **Plan, schedule and compare bounded same-start branch experiments.** A new
  `benchmark_manifest::branch_experiment` module fixes the effect-free contract behind issue #119: a
  versioned declaration of one verified fork point, a fork strategy, child policies and per-child and
  total budgets; a stable per-child trial key with its own fresh provider/context namespace; a
  same-start admission re-check that keeps a prefix-only start out of exact-restore statistics; a
  retry-safe recorded scheduler that reconciles a lost reply without double-scoring a trial; an
  aligned comparison that separates declared policy divergence from restore failure and does not let
  an identical endpoint erase an earlier divergence; and a sanitized report carrying a keyed handle
  and no exact digest. Source-only
  ([ADR 0072](decisions/0072-branch-experiment-comparison.md)): the live children, the restore
  and the provider calls stay with the gateway and game-mod. Refs #119.

- **Resolve and durably bind an authored workflow's seed.** A new `seed_binding` module fixes the
  source-only contract behind #103: an explicit or generate-once seed normalizes to one bounded
  canonical UTF-8 form, a generate-once run draws at most once per persisted record and the persisted
  effective seed is reused across duplicate requests, lost responses and restarts without a redraw
  (only a retry after a failed persist may redraw, before any record exists), the effective seed is
  persisted before any setup mutation (failing closed), and a wrong instance, stale baseline or lease,
  unsupported setup, or conflicting persisted seed is refused before any draw. A recording transport proves the
  persisted effective seed and operation identity are sent unchanged. Native seed acceptance stays
  gated by sts2-game-mod#79
  ([ADR 0071](decisions/0071-authored-seed-binding.md)). Refs #103.

- **Refuse a suite whose case and policy axes cannot derive a settleable trial key.** `SuiteManifest`
  bounded a `case_id` and a `policy_id` separately at `MAX_SUITE_LABEL_BYTES` (128), while
  `TrialOutcome::validate` refuses a `trial_key` over `MAX_TRIAL_KEY_BYTES` (256) and the key
  concatenates both labels around a 64-hex suite revision. A manifest that validated could therefore
  plan a trial whose outcome `settle` refused forever. The combined pair is now bounded by a derived
  `MAX_SUITE_TRIAL_AXIS_BYTES`, so every accepted manifest is plan-and-settleable, and an oversized
  single id is still refused as an invalid label. Source-only: no released artifact was affected and
  no live caller reached the case. Compatibility: an input that previously validated and then failed
  at settlement is now refused at validation.

- **Admit a bounded pre-agent read-only recipe.** A new `recipe` module fixes the source-only
  contract behind #97: an authored workflow may declare a bounded, versioned recipe of approved
  read-only tool reads that the harness admits before provider dispatch, with a fixed refusal order,
  a declared topological step order with no cycles or forward references, and mutation tools refused
  from the read-only catalog ([ADR 0073](decisions/0073-pre-agent-read-only-recipe-admission.md)).
  Collection execution, provenance and the Studio round-trip remain open. Refs #97.
