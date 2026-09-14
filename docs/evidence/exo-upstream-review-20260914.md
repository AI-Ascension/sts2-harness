# Evidence: reviewed Exo upstream delta `7801005..b06869a` (2026-09-14)

Issue: AI-Ascension/sts2-harness#139. Tracker: #138.

This reviews the nine commits between the previously checked-in audit pin
`7801005e6a1ab77008a05dbba80e0a2a7a56e35d` and the selected candidate
`b06869ab789dee3f80ca474b5fa89dbe47ccb859`, and freezes the reviewed source/manifest. The review is
`source-derived`; it is not live execution evidence.

## Reviewed delta

| # | Commit | Subject | Touched surface (relevant paths) | Effect on the chosen deployment |
|---|---|---|---|---|
| 1 | `8397544` | Add Firecracker backend reset API (#255) | `crates/exoharness/src/sandbox_provider/firecracker.rs` | Out of scope: Firecracker-only sandbox backend. |
| 2 | `36b2b81` | Add persistent Firecracker terminals (#260) | `exoharness/src/sandbox.rs`, `sandbox_provider/{firecracker,process_bridge}.rs`, `types.rs` | Out of scope for `local-process`; adds sandbox terminal types. Does not touch executor send or the TypeScript harness. |
| 3 | `d76bd01` | Bound Firecracker snapshot storage and add safe deletion (#263) | `exoharness/src/sandbox.rs`, `sandbox_provider/firecracker*.rs` | Out of scope: Firecracker snapshot storage. |
| 4 | `cc4461e` | Reconnect ExoChat after WebSocket drops (#262) | (web/chat frontend) | Out of scope: human-facing chat UI, not the headless bridge. |
| 5 | `29ebbcd` | Reduce Firecracker network setup and termination latency (#266) | `sandbox_provider/firecracker.rs` | Out of scope: Firecracker latency. |
| 6 | `a153bf6` | Separate sandbox identity from owner scope and trace Firecracker lifecycle (#267) | `crates/exoharness/src/basic.rs`, `basic_tests.rs`, `sandbox` + CLI sandbox tests | Partially relevant: separates sandbox identity from owner scope. Not exercised by the `local-process` spike; relevant to sandbox-backed profiles and to instance-identity separation in the bridge contract. |
| 7 | `87bfc53` | Preserve Firecracker image configuration and speed up materialization (#272) | `exoharness/Cargo.toml`, `sandbox_provider/firecracker*.rs` | Out of scope: Firecracker image materialization. |
| 8 | `c5c1963` | Capture sparse Firecracker snapshots with cumulative memory bases (#274) | `sandbox_provider/{firecracker,firecracker_tests}.rs` | Out of scope: Firecracker snapshots. |
| 9 | `b06869a` | fix: correct DEFERRED_SCRIPT path in guardian-tools (#233) | guardian-tools | Out of scope for the bridge; sandbox tooling fix at the candidate tip. |

No commit in the delta modifies `crates/executor` (`HarnessConversation::send`, `SendRequest`/
`SendResult`, `exo_from_root`), `exoharness/typescript/model-runtime/*`, or `exo/harness.ts`. The
chosen executor embedding path is therefore unchanged across the delta; the drift is confined to
Firecracker/sandbox backends and the chat UI that the `local-process` bridge does not use.

## Reviewed source/package manifest

| Field | Value |
|---|---|
| `exo_source_revision` | `b06869ab789dee3f80ca474b5fa89dbe47ccb859` |
| Previous audit pin | `7801005e6a1ab77008a05dbba80e0a2a7a56e35d` |
| `exo_executable_digest` | `eef56bfb39f67f7c616284547ad7a8acf364f897a95ce91065e7174b966c46f8` (debug build captured by the spike; a different build changes the digest) |
| `sts2_fixture_sha256` | `119daeaefad4897463afa2e83d238154d317f4781371fa23e34e7bf513900b41` |
| `sts2_driver_sha256` | `f80f4d3765deed7dccfaac4bc9d4da88076d4bf544cb31a23a4ff81f1f0002a1` |
| Exo toolchain | Node `22.15.0`, pnpm `10.26.2` (upstream `mise.toml`) |
| `model_binding` | `gpt-test` → synthetic endpoint (no real credential) |
| `contract_version` | `sts2.exo-bridge-v1` |

## Evidence states

- `source-derived`: the per-commit classification above, from the pinned clone.
- `confirmed`: the delta count is nine and none of the nine touches the chosen executor path.
- `unverified`: sandbox-backed (Firecracker/Daytona/E2B/etc.) deployment behavior; this review did
  not build or run those backends.
- `proposed`: `sts2.executable_digest` binding must be supplied by trusted operator configuration,
  not self-reported by the bridge.
