# Harness main-integration conflict resolution

Date: 2026-09-09. Classification: documented local merge, build, and test evidence.
This record is a handoff for review; it is not a claim of remote publication,
merge, installation, or live runtime settlement.

## Merge identity and evidence locations

- Worktree: isolated integration checkout; personal paths are intentionally omitted.
- Branch: `codex/harness-main-integration`
- Worker-side `HEAD`: `a8ce56a9` (`Verify Linux endpoint consumer against pinned owner vectors`)
- Main-side `MERGE_HEAD`: `e72de4d5` (`Merge pull request #38 from AI-Ascension/recovery/harness-g3-current-main-integration-20260908`)
- Cargo target directory: dedicated ephemeral directory, represented below by
  `$HARNESS_INTEGRATION_TARGET`.
- Durable evidence record: `docs/evidence/harness-main-integration-20260909.md`
- Separate shell transcripts were not redirected to files. The target directory
  contains Cargo build/test artifacts, not a durable test log; this document is
  the durable summary of the observed exits and counts.

Before resolution, `git ls-files -u` reported 54 index entries covering 21
unique unresolved paths. The working tree was deliberately left in merge state
for parent review; no source conflict was staged by this audit.

## The 21 conflict decisions

Each path below is the selected hybrid behavior in the current working tree.
These were the initial resolution intentions, not verified integrated behavior.
Root review found that active episode dispatch and recovery had lost durable
wiring; the candidate must not be published until that regression is repaired.

| # | Path | Resolution decision |
|---:|---|---|
| 1 | `crates/harness/src/bin/runtime_support/mcp_process.rs` | Keep main's typed MCP transport errors and bounded/graceful close; retain the worker branch's cancellation-aware process/recovery spawn behavior. |
| 2 | `crates/harness/src/bin/runtime_support/mod.rs` | Keep the complete main module graph and place the worker admission/transport gate before profile selection, so worker policy is established before provider profiles are used. |
| 3 | `crates/harness/src/bin/runtime_support/runtime_v3.rs` | Route Runtime-v3 gameplay through the durable `open` plus execution path and retain the Runtime-v4 expert `run_legacy` composition entrypoint. |
| 4 | `crates/harness/src/bin/runtime_support/runtime_v3_combat_demo.rs` | Preserve bounded combat execution, `EpisodeShutdown`, telemetry/report cleanup, and replay observation digest behavior. |
| 5 | `crates/harness/src/bin/runtime_support/runtime_v3_episode.rs` | Keep main's typed MCP/expert composition and episode semantics; use cancellation-independent `GatewayClient::release` for allocation cleanup so cancellation cannot skip lease release. |
| 6 | `crates/harness/src/bin/runtime_support/runtime_v3_episode_replay.rs` | Keep bounded pre-parse input handling and the worker-side replay digest/rebind checks. |
| 7 | `crates/harness/src/bin/runtime_support/runtime_v3_lifecycle_test.rs` | Combine main allocation/lifecycle tests with worker reconnect, recovery, and quarantine coverage. |
| 8 | `crates/harness/src/bin/runtime_support/runtime_v3_port.rs` | Retain main's Runtime-v3 expert port while adding durable operation/worker state and the `ShutdownPort` cleanup boundary. |
| 9 | `crates/harness/src/bin/runtime_support/runtime_v3_recording.rs` | Retain worker `DecisionRecorder` and provider reservation/accounting while exporting the main telemetry records. |
| 10 | `crates/harness/src/bin/runtime_support/runtime_v3_recovery.rs` | Combine reconnect/recovery context and durable operation records with main sideband recovery reporting. |
| 11 | `crates/harness/src/bin/runtime_support/runtime_v3_telemetry.rs` | Preserve main's FIFO/OTLP exporter module graph. |
| 12 | `crates/harness/src/bin/runtime_support/runtime_v3_telemetry_render.rs` | Preserve main's bounded redacted rendering contract. |
| 13 | `crates/harness/src/bin/runtime_support/runtime_v3_telemetry_tests.rs` | Preserve main's redaction, timestamp, sequence, and export-status tests. |
| 14 | `crates/harness/src/bin/runtime_support/runtime_v3_telemetry_types_a.rs` | Preserve main's sanitized context and finite event classifications. |
| 15 | `crates/harness/src/bin/runtime_support/runtime_v3_telemetry_types_b.rs` | Preserve main's bounded exporter/status types and enqueue sequence metadata. |
| 16 | `crates/harness/src/bin/runtime_support/runtime_v3_telemetry_types_c.rs` | Preserve main's serialized FIFO admission and bounded nonblocking enqueue. |
| 17 | `crates/harness/src/bin/runtime_support/runtime_v3_telemetry_types_extra.rs` | Preserve main's extra finite telemetry types. |
| 18 | `crates/harness/src/bin/runtime_support/runtime_v3_telemetry_worker.rs` | Preserve main's ordered export and separate final export-status event. |
| 19 | `crates/harness/src/bin/runtime_support/runtime_v3_wait.rs` | Keep typed gateway/MCP failure handling and add worker recovery, digest, and action-settlement wait helpers. |
| 20 | `crates/harness/src/bin/runtime_support/runtime_v3_wire.rs` | Keep main typed wire/gateway failure semantics and add worker recovery, digest, action, and terminal-state helpers. |
| 21 | `crates/harness/src/lib.rs` | Export main Runtime-v4/expert APIs together with worker cancellation, durable control, and accounting APIs. |

The last episode cleanup adjustment was verified after changing the cleanup
closure from `gateway.request` to cancellation-independent `gateway.release`.
This is important to review as a resource-safety decision, not merely a type
check repair.

## Changed companion files

These are the non-conflict files changed by the two sides and retained in the
integration candidate.

MCP and worker transport:

`crates/harness/src/bin/runtime_support/config.rs`,
`crates/harness/src/bin/runtime_support/mcp_process_error.rs`,
`crates/harness/src/bin/runtime_support/mcp_process_spawn.rs`,
`crates/harness/src/bin/runtime_support/mcp_process_tests.rs`,
`crates/harness/src/bin/runtime_support/mcp_process_transport.rs`.

Runtime-v3 durable, replay, lifecycle, and wire companions:

`runtime_v3_allocation_launch_test.rs`, `runtime_v3_combat_replay.rs`,
`runtime_v3_combat_replay_tests.rs`, `runtime_v3_decision_admission.rs`,
`runtime_v3_execution.rs`, `runtime_v3_execution_tests.rs`,
`runtime_v3_ledger.rs`, `runtime_v3_lifecycle_expert_catalog_runner_test.rs`,
`runtime_v3_lifecycle_fault_matrix_test.rs`,
`runtime_v3_lifecycle_recovery_test.rs`,
`runtime_v3_lifecycle_runner_fixture.rs`,
`runtime_v3_lifecycle_runner_test.rs`, `runtime_v3_parse.rs`,
`runtime_v3_replay_digest.rs`, `runtime_v3_wire_failure.rs`,
`runtime_v3_wire_recovery.rs`, `runtime_v3_wire_stage.rs`,
`runtime_v3_wire_tests.rs`, `runtime_v3_worker_runtime_execution.rs`, and
`runtime_v3_worker_store_tests.rs`, all under
`crates/harness/src/bin/runtime_support/`.

Runtime-v4 expert and protocol consumers:

`crates/harness/src/runtime_v4_expert.rs`,
`runtime_v4_expert_action.rs`, `runtime_v4_expert_action_artifact.rs`,
`runtime_v4_expert_action_strict.rs`, `runtime_v4_expert_action_types.rs`,
`runtime_v4_expert_action_validation.rs`, `runtime_v4_expert_artifact.rs`,
`runtime_v4_expert_parse.rs`, `runtime_v4_expert_shape_actions.rs`,
`runtime_v4_expert_shape_collections.rs`, `runtime_v4_expert_shape_root.rs`,
`runtime_v4_expert_types_a.rs`, `runtime_v4_expert_types_b.rs`,
`runtime_v4_expert_validation.rs`, and the port files
`runtime_v4_expert_port.rs`, `runtime_v4_expert_port_composition.rs`,
`runtime_v4_expert_port_tests.rs`, `runtime_v4_expert_port_transport.rs`,
`runtime_v4_expert_port_transport_composition.rs`, and
`runtime_v4_expert_port_transport_receipt.rs`.

Bridge, accounting, episode, and integration tests:

`crates/harness/src/bin/sts2-astra-bridge.rs`,
`crates/harness/src/bin/support/bridge_accounting.rs`,
`crates/harness/src/exo/codex_accounting.rs`, `crates/harness/src/exo/mod.rs`,
`crates/harness/src/exo/sandbox.rs`, `crates/harness/src/exo/sandbox_api.rs`,
`crates/harness/src/exo/sandbox_validator.rs`,
`crates/harness/src/episode/legal_actions.rs`,
`crates/harness/src/episode/recovery.rs`,
`crates/harness/src/episode/runner_steps.rs`,
`crates/harness/tests/codex_accounting.rs`,
`crates/harness/tests/episode_runner.rs`,
`crates/harness/tests/episode_runner/catalog_reobserve.rs`,
`crates/harness/tests/episode_runner/scenarios.rs`,
`crates/harness/tests/runtime_v4_executable_composition.rs`,
`crates/harness/tests/runtime_v4_expert_action.rs`,
`crates/harness/tests/runtime_v4_expert_artifact.rs`,
`crates/harness/tests/support/runtime_v4_executable_composition_fixture.rs`, and
`crates/harness/tests/support/runtime_v4_executable_composition_process.rs`.

CI, documentation, conformance, and copied protocol artifacts:

`.github/workflows/ci.yml`, `RELEASING.md`,
`docs/decisions/0009-seeded-episode-replay.md`,
`experiments/live-combat/README.md`,
`conformance/cases/runtime-v4-expert.json`,
`conformance/cases/runtime-v4-expert-action.json`,
`schemas/runtime-v4-expert.schema.json`,
`schemas/runtime-v4-expert-action.schema.json`, and the complete
`protocol-artifact/runtime-v4-expert/` and
`protocol-artifact/runtime-v4-expert-action/` trees (README, manifest, schema,
golden payloads, and `SHA256SUMS`).

## Executed gates and exact observed outcomes

All commands used the dedicated target directory below. Commands shown with
`cargo` were run from the integration worktree.

```text
CARGO_TARGET_DIR="$HARNESS_INTEGRATION_TARGET" cargo check --locked --package sts2-harness -j 2
exit 0 (approximately 109 warnings)

CARGO_TARGET_DIR="$HARNESS_INTEGRATION_TARGET" cargo test --locked --package sts2-harness --bin sts2-harness-runtime --no-run -j 2
exit 0

CARGO_TARGET_DIR="$HARNESS_INTEGRATION_TARGET" cargo test --locked --package sts2-harness --test completed_resume_process -- --nocapture
exit 0; 7 passed, 0 failed

CARGO_TARGET_DIR="$HARNESS_INTEGRATION_TARGET" cargo test --locked --package sts2-harness --test runtime_startup -- --nocapture
exit 0; 5 passed, 0 failed

CARGO_TARGET_DIR="$HARNESS_INTEGRATION_TARGET" cargo test --locked --package sts2-harness --test worker_server_entry -- --nocapture
exit 0; 5 passed, 0 failed (after the release-cleanup adjustment)
```

The post-adjustment all-targets matrix was run twice with:

```text
CARGO_TARGET_DIR="$HARNESS_INTEGRATION_TARGET" cargo test --locked --package sts2-harness --all-targets -j 2
```

Both runs exited 101. The only reported failure was the parallel Linux worker
test `worker_local_linux::wrong_uid_gid_pid_start_and_image_fail_before_credential_read`,
which returned a bare `Error: Io`; the other targets completed successfully.
This is not recorded as a green full matrix.

Isolation checks for the same target and source both passed:

```text
CARGO_TARGET_DIR="$HARNESS_INTEGRATION_TARGET" cargo test --locked --package sts2-harness --test worker_local_linux -- --nocapture
exit 0; 25 passed, 0 failed
```

The focused rerun of the failing test also exited 0. The evidence supports a
parallel-only test fixture/verifier contention diagnosis, but does not prove
the root cause or waive a CI repair. A reviewer should preserve the failed
parallel result and require a reproducible isolation-safe fix before calling
the all-targets gate green.

No host/game/provider launch, VM test, service installation, remote push, PR
creation, merge, release activation, or live gameplay settlement is evidenced
by these commands.

## Root review: integration remains blocked

The initial candidate's active `EpisodeRuntimePort::dispatch_action` retained
only an in-memory operation before sending; durable intent and dispatch
uncertainty methods were not called. Allocation bypassed the new recovery
authority validator, and `RecoveryPort::reconcile` used the legacy gameplay
recovery path while historical-sideband methods were unreachable. These are
source-confirmed integration defects, not unused-code cleanup opportunities.

Root validation before repair, with two build jobs and the same dedicated target:

```text
cargo test --locked -p sts2-harness --bin sts2-harness-runtime runtime_v3_telemetry -- --test-threads=8
exit 0; 8 passed

cargo test --locked -p sts2-harness --bin sts2-harness-runtime runtime_v3_wire -- --test-threads=8
exit 0; 13 passed

cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
exit 101; runtime binary failed with 99 errors, largely unreachable durable/recovery code
```

Those focused tests validate their bounded modules only. They do not establish
durable dispatch, historical reconciliation, or complete merge correctness.

Root also restored omitted module declarations for the original allocation,
original-context, durable reconnect, recovery-evidence, MCP cancellation, and
worker-store regressions. Before repair, restoring the lifecycle tests exposed
seven compile errors (removed initializer API, duplicated fixture field,
non-fallible observation installation, and missing pending reconciliation).
The initializer calls and duplicate fixture fields were corrected without
weakening assertions; durability-path errors require the production repair.
An earlier passing test run that omitted these modules is not full coverage.

Two responsibility-preserving extractions address strict size checks: the
existing Runtime-v4 executable entrypoint is included from
`runtime_v4_execution.rs`, and episode-runner synthetic state/configuration
builders live in `episode_runner/fixtures.rs`. The episode-runner test target
passed all 19 tests after extraction. Strict policy then reported only the
remaining oversized `runtime_v3_port.rs` (345 nonblank lines, preferred limit 300).

## Restored runtime regression coverage

The first complete runtime-unit run after restoring the omitted modules ran
189 tests: 184 passed and five failed. Two failures were synthetic directory
collisions between independently counted legacy and durable reconnect fixtures;
the legacy fixture now uses a distinct UUID-based directory namespace. Three
historical recovery failures stopped after lookup because an MCP `isError`
envelope was discarded before the strict historical-recovery parser could
validate its unresolved result. The recovery-only RPC path now preserves that
envelope for validation; ordinary gameplay errors retain their prior handling.

Root reran the combined runtime suite after these repairs:

```text
cargo test --locked -p sts2-harness --bin sts2-harness-runtime -j 2 -- --test-threads=8
exit 0; 189 passed, 0 failed; 36.68 seconds
```

This run still reported two unused port wrappers, so it is not a passing
warnings-as-errors gate. Independent source review and the complete workspace
matrix remain pending. The merge is still unstaged and unpublished.

Root's subsequent complete Linux matrix exited 0:

```text
CARGO_TARGET_DIR="$HARNESS_INTEGRATION_TARGET" cargo test --workspace --all-targets --all-features --locked -j 2
exit 0; runtime unit suite 189 passed; worker_local_linux 25 passed
```

The operator-only Runtime-v4 executable-composition test remained explicitly
ignored because this command did not supply the required separately built
gateway/MCP binaries. This is not a passing cross-repository composition result.
The Linux worker suite included the previously failing peer-identity case and
finished in 81.35 seconds. Its test-only serial guard now retains ownership
until the exact poisoned child has been reaped, with a bounded two-second
cleanup deadline and visible failure. Production singleton ownership and
fail-closed cleanup behavior are unchanged. This matrix does not waive the
remaining warnings, independent review, or the combat/full-episode completion
scope issue identified during review.

## Independent review after the green test run

A separate reviewer identified two uncovered in-process runner paths. An
unresolved historical reconcile returned an `Unknown` receipt; the generic
runner then entered its transition barrier, and the durable port permitted a
normal gameplay wait to reconcile that unknown record. This violates ADR 0011's
historical-only unresolved recovery rule. Separately, a historically settled
operation returned no successor observation, so the runner rejected it with
`MissingObservation` after durable closure. Existing recovery-evidence tests
exercised startup reconciliation, not this complete in-process runner sequence.

Root confirmed both call paths by source inspection. Publication remains held
pending regressions and a repair that preserves unknown outcomes, prohibits
gameplay polling before historical closure, and requires valid same-incarnation
continuation evidence after closure. The prior green workspace result is retained
as test evidence, not treated as proof that these paths were safe.

## Completion ordering and workflow binding follow-up

The runner now offers a completion callback after a validated terminal report
and before lease/MCP/gateway cleanup. The ordinary entrypoint supplies a no-op
callback. Completion or execution failure still attempts cleanup, and a combined
failure preserves both causes. Root independently ran:

```text
cargo test --locked -p sts2-harness --test episode_runner completion_ordering -j 2
exit 0; 5 passed; 0.05 seconds
cargo test --locked -p sts2-harness --bin sts2-harness-runtime workflow_binding -j 2
exit 0; 6 passed; 0.84 seconds
```

The first target covers exactly-once completion, failure suppressing completion,
completion persistence failure, cleanup failure after completion, and combined
execution/cleanup failure. The second covers distinct workflow/replay bindings,
cross-mode durable resume rejection, combat-only Reward completion, generic
Reward rejection, and bounded replay input. The standalone binding uses captured
replay bytes; worker fingerprint compatibility is separately tested.

A responsibility-preserving extraction moved durable completion into its own
module. The initial extraction did not compile because of two module/import
references; root corrected those before the passing workflow-binding run above.
Independent review also found a missing combat completion output event; its
repair and tests are pending. These focused results are not a final integrated
workspace, historical-recovery, or native Windows harness pass.

## Integrated validation and native composition follow-up

After the recovery and completion repairs, root ran the complete local gates:

```text
cargo fmt --all -- --check
cargo run --locked --package repo-policy -- --strict
cargo clippy --workspace --all-targets --all-features --locked -j 2 -- -D warnings
cargo test --workspace --all-targets --all-features --locked --no-fail-fast -j 2
```

All four commands exited 0. The strict policy scan covered 623 files with no
warnings or errors. The workspace reported 537 passing tests across 51 targets,
zero failures, and one explicitly ignored executable-composition test. The
completed-resume and corrupt-operation startup fixtures were updated to include
the full-episode/no-replay workflow descriptor; their original no-external-call
and corruption-rejection assertions remain intact. Their focused suites passed
7 and 5 tests respectively. A test-owned fingerprint helper was extracted to
remain within the repository size limit.

This green run predates subsequent native composition and telemetry findings.
The real watchdog stages immutable controller and harness images and executes
the harness through its Linux sealed-executable launch adapter. The smoke did
not reach authenticated worker admission: the worker exited with a native
transport error. Source inspection identified its fixed verifier launch using
`current_exe()` as a reopenable filename, incompatible with a sealed memfd
execution image. A kernel-bound self-reexecution repair and an actual memfd
regression are required before claiming that composition passes. This is not
evidence of gameplay, provider execution, service installation, or deployment.

Independent review also found combat telemetry conflating successful cleanup
with the authoritative terminal game outcome. Its correction must retain the
actual Defeat/Reward/Victory stage independently of provider/store cleanup and
leave unavailable outcomes unavailable. Final publication remains held until
the new repairs and a fresh integrated gate pass.

## Post-repair Linux gate and remaining admission failure

Root reran format, strict policy, warnings-as-errors Clippy, and the complete
workspace/all-target/all-feature locked test matrix after the combat telemetry
repair and fixed verifier extraction. All four commands exited 0. Strict policy
checked 626 files with no warnings or errors. The Runtime-v3 binary suite passed
208 tests, the library suite passed 79 tests, and the native Linux worker suite
passed 27 tests. The operator-only Runtime-v4 composition test remained ignored.

Independent review approved the private `/proc/self/exe` self-reexecution seam:
production environment clearing, fixed arguments, CLOEXEC descriptor handling,
sealed-image identity, bounded image copy, and owned child cleanup remain intact.
The reviewer independently passed nine verifier tests, two verifier-entry tests,
and two protected-image tests. These are synthetic native process results, not
service installation, gameplay, or provider evidence.

The exploratory release binary with SHA-256
`0cb1d6a693f33cb48437b5e18885042b8ad840c61ccc061da4633fb0470149d9`
then reached real authenticated worker Running control from the watchdog's
native launcher. The smoke still failed: its submitted job remained queued for
the bounded observation period, with no durable handoff prepared. The watchdog
persisted stop and cleaned its worker; failed-run evidence was retained privately.
The artifact predates the final source commit and is not a release-set artifact.
Admission diagnosis and a fresh exact-byte composition run remain required.

Subsequent source inspection and a second native reproduction located that
admission failure in the watchdog: its live-component observation supplied no
heartbeat, so policy changed the worker to Suspect and refused fresh claims.
The harness's authenticated Running control was retained. The watchdog repair
must consume a fresh authenticated worker observation without equating process
liveness with health. The harness gate results above remain valid; complete
watchdog-to-harness job admission remains unverified until a fresh run passes.
