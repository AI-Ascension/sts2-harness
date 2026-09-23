# ADR 0068: Isolate cold-launch benchmark trials behind one pristine baseline contract

Status: accepted for the harness-owned, source-only slice of issue
[#122](https://github.com/AI-Ascension/sts2-harness/issues/122) — an immutable baseline, exclusive
destination leases, attested process births with readiness proofs, a recorded stage machine, lost
reply reconciliation and machine-readable cold-start evidence. It launches no game, mutates no
profile and spends no provider credit: the effect-performing port belongs to the gateway
(sts2-gateway#50/#51) and native acceptance stays gated by sts2-game-mod#79. It is ratified when the
change carrying it merges.

## Context

Issue #122 requires every benchmark trial to start in a newly identified native game process from
the same immutable baseline, so earlier trials, saves, unlocks and process-static state cannot
contaminate the next trial. Integrated acceptance is blocked by native prerequisites, but the triage
record explicitly allows contract design, a source audit and effect-free fixtures to proceed first.
The harness already owns benchmark declarations ([ADR 0021](0021-benchmark-manifest-foundation.md))
and suite scheduling ([ADR 0067](0067-reproducible-benchmark-suite-scheduling-and-reports.md)); what
was missing was the per-trial isolation contract those layers schedule against.

Four failure modes had to be excluded by construction:

1. provisioning the next trial from the previous trial's result instead of the preserved baseline;
2. treating a controller restart, an in-process seed reset or a stale resume as a cold launch;
3. handing one writable destination to two trials, or reusing an uncertain one;
4. reporting a cleanup failure as a victory or a defeat.

## Decision

A new module `crates/harness/src/benchmark_manifest/cold_launch/` owns that contract, split so each
file stays inside the production size budget:

- **Immutable baseline (`baseline.rs`).** `PristineBaseline` binds the baseline artifact digest, a
  `LaunchProfile` (profile id, build digest, game version) and a closed, exact
  `TelemetryExclusions` list. `validate` bounds and rejects NUL-bearing labels; `compare` lists every
  differing category in a stable order and `is_compatible` is exactly "no differing category", so
  exclusions can never be widened to force equality.
- **Process identity (`process.rs`).** `ProcessBirth` is an opaque gateway-attested token plus a
  strictly positive instance generation, because a PID can be reused. `ReadinessProof` re-validates
  its own birth and refuses a proof whose generation disagrees with that birth (`StaleReadiness`).
- **Exclusive leases (`lease.rs`, `orchestrator.rs`).** A bounded `LeaseAllocator` never hands one
  destination to two trials and treats a repeat lease to the same trial as idempotent. The
  orchestrator adds a live-birth registry (a token is live for exactly one trial) and a permanent
  quarantine set, and refuses to lease a quarantined destination.
- **Stage machine (`stage.rs`, `lifecycle.rs`).** `TrialLifecycle` admits only against a validated
  baseline, and `admit_against` also refuses a declared baseline that differs from the admitted
  reference in any category before action admission. Each stage records its effect before the next:
  reserve, provision, launch, prove readiness, settle setup, run, stop, clean. A lost reply is
  idempotent for the recorded birth, refused for another birth, and adopted only from `Provisioned`,
  so a restart cannot silently become a launch. `CleanupFailed` and `Quarantined` are terminal and
  keep the destination from reuse, and cleanup failure is a separate field from the gameplay outcome.
- **Evidence (`evidence.rs`, `error.rs`).** `evidence_of` emits `ColdStartEvidence` (trial key, stage,
  baseline digest, birth generation and token, held destination, cleanup flag, mismatch reasons) for
  the verifier and the suite scheduler. `ColdLaunchError` is the bounded rejection vocabulary; no
  supplied field value, seed or digest is reflected in a message.

## Consequences

- Two serial trials provably differ in birth token and instance generation while sharing one
  baseline digest, because identity travels on the attested birth rather than on a process handle.
- A mutated profile cannot leak, because a trial holds the admitted baseline immutably and the next
  admission is compared against the preserved reference.
- A crash, cancel or lost reply cannot double-allocate or adopt another trial's state, because
  leasing is exclusive and bounded, reconciliation is identity-bound and quarantine is permanent.
- Cost: three small registries (leases, live births, quarantine) and one extra stage. The
  alternative — inferring isolation from a PID or a destination path — was rejected because neither
  distinguishes a reused identity from a fresh one.

## Validation

- `crates/harness/tests/cold_launch.rs` covers two serial trials with distinct births against one
  unchanged baseline, a mutated first-trial profile that cannot leak, every baseline mismatch
  category refused before admission, a reused live process, a foreign or shared destination, stale or
  forged readiness, adoption of a lost reply only from `Provisioned`, failure injection at each stage
  that changes nothing and never rewrites the baseline, cleanup failure reported apart from the
  outcome, cancellation that keeps evidence, bounded declarations and machine-readable evidence.
- `crates/harness/tests/cold_launch_lease.rs` covers exclusive and bounded concurrent leases, an
  idempotent same-trial lease, a released slot, a quarantined destination that is never re-leased,
  holder reporting and out-of-range allocation bounds.
- Source-only: no native run, provider call or profile mutation is exercised; the native cold-launch
  witness and the real child-process lifecycle lane remain unverified until their gates record
  evidence.

## References

- Issue [#122](https://github.com/AI-Ascension/sts2-harness/issues/122): "Orchestrate isolated
  cold-launch benchmark trials from pristine profile baselines".
- [ADR 0021](0021-benchmark-manifest-foundation.md): private benchmark declarations reused here.
- [ADR 0067](0067-reproducible-benchmark-suite-scheduling-and-reports.md): the suite scheduler that
  schedules these trials.
