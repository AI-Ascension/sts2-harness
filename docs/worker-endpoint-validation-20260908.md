# Linux worker endpoint namespace validation

Classification: confirmed component and synthetic executable evidence, 2026-09-08.
Branch: `codex/harness-worker-auth-bridge`; base
`c3240cff40b5247c533c94fb1011cdfdcdfa4eff`.

The Linux listener derives its socket from the validated startup nonce and static
`STS2_WORKER_ENDPOINT_NAMESPACE`. Any legacy `STS2_WORKER_ENDPOINT` presence is
rejected. Namespace resolution is pure and does not authorize stale-socket removal.
Independent review confirmed matching watchdog/Linux harness derivation semantics.

Executed successfully using the pinned toolchain:

- `cargo fmt --all --check`.
- `cargo run --locked --package repo-policy -- --strict`: 557 sized files,
  no warnings or errors before this evidence document was added.
- `cargo test --locked -p sts2-harness --test worker_endpoint_linux --test worker_server_entry -- --test-threads=1`:
  two endpoint tests and five server-entry tests passed; server suite elapsed
  121.03 seconds, including fixture setup and cleanup.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
- `cargo test --workspace --all-targets --all-features --locked -- --test-threads=1`.

The initial fixture directory prefix exceeded the socket-path bound after adding
the launch nonce. The test-only prefix was shortened without relaxing production
validation. The original five-second startup wait timed out, including serially;
a bounded 30-second test startup wait passed. Production exchange and shutdown
deadlines and explicit shutdown assertions were not changed. Existing owned-child
cleanup was moved to a dedicated test-support module to preserve policy size limits.

These tests launch the synthetic harness executable with a test-supplied bootstrap.
They do not establish a watchdog-produced bootstrap, Windows listener integration,
service installation/recovery, live game/provider execution, reboot, or soak.
Exact release-set integration and shared machine-readable cross-consumer vectors
remain separate gates.
