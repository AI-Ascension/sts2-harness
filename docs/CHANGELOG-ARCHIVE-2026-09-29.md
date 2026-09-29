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
- **A transport that refuses a request is reported as the refusal, not as a broken pipe.**
  #746 made the bridge print `cause: {error}`, but the workers were joined before the transport's
  exit status was read, and a transport that exits without draining its stdin fails the writer's
  `write_all` with `EPIPE` — so `??` returned on that plumbing artifact and the `status.success()`
  check never ran, so an operator transport declining a request with a non-`200` was reported as
  `cause: Broken pipe (os error 32)`. The exit status does not depend on scheduling, so it is read
  first; the workers are still joined, so a real transport I/O failure is still surfaced. New case
  `a_refused_transport_is_reported_as_the_refusal_and_not_as_a_broken_pipe` asserts the cause is the refusal and is not `Broken pipe`. Refs #751.
- **A transport that cannot be given a worker thread is killed, not orphaned.**
  #746 made the bridge report a failed thread spawn instead of panicking, and #747 then killed the
  child when the *reader* worker could not start. The writer arm was left returning the error with a
  plain `?`, and the transport is already running by then: `std::process::Child` has no `Drop` that
  signals the process, so dropping it closes the handles and leaves the child alive. A host that
  could not give the bridge its *first* worker thread therefore reported the failure correctly and
  still orphaned the transport. Both arms now go through one `kill_child` helper, and the
  regression tests assert the process table rather than the error string, which is byte-identical
  either way. Each arm's case drives a refusal through the real `exchange`, with a real child and
  real pipes, and reverting *either* arm to a plain `map_err` leaves the transport running and
  fails that arm's case — so the wiring is asserted, not merely the helper. Refs #748.
