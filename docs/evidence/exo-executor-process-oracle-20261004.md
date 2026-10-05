# Exo process evidence refresh — 2026-10-04

These records exercise the shipped Exo bridge and pinned Exo with a synthetic loopback model.
They do not exercise a real provider, game, native host, or full runtime admission.

## One-shot process run

[`exo-executor-process-oracle-20261004.json`](exo-executor-process-oracle-20261004.json) is a
byte-for-byte copy of the successful bounded process report retained at
`orchestration/tasks/harness109-delivery/exo-smoke-report-eddc511.json` (SHA-256
`70ac7d1b7bd97330f158de3247672644f5bf9ca348b002c5f98f21698c0c88af`). It records harness source
revision `eddc511a31c034978661d7aca6829d3f92cf9e0b`, Exo revision
`b06869ab789dee3f80ca474b5fa89dbe47ccb859`, and the bridge binary exercised at SHA-256
`97985d0d77cb0ab70765d7f176a99ec4c442fac1a314d5784ed0f98c9cb29e70`.

The report contains 42 passing cases. `old_history_sentinel` records refusal before model egress
with zero requests. `synthetic_v2_success` records one request and a present receipt. The reported
extension, process-oracle, and support-module digests match the corresponding files at the current
source head; the report retains its actual earlier run revision and binary digest.

## Advertised-variant process run

[`exo-advertised-variant-negatives-20261004.json`](exo-advertised-variant-negatives-20261004.json)
was emitted by the current-source ignored process test
`advertised_variants_and_zero_model_probes` at harness revision
`03f0422fa11c7110e5479ce6383dc318a53b4c65`. The Cargo test portion was:

```sh
STS2_EXO_TEST_NODE="$NODE_BIN_DIR/node" \
STS2_EXO_TEST_SOURCE="$PWD/target/exo-source" \
CARGO_TARGET_DIR="$PWD/target/exo-executor" CARGO_BUILD_JOBS=1 \
  cargo test --locked --manifest-path experiments/exo-agent/bridge/Cargo.toml \
  --test advertised_variant_oracle -- --ignored --exact \
  advertised_variants_and_zero_model_probes --nocapture
```

That block shows the Cargo test portion, not the complete WSL invocation. The actual command also
used a task-local Git wrapper and explicit `HARNESS_WORKTREE`, `HARNESS_GIT_DIR`, Node, Exo source,
and owned target values. Their resolved values and complete argument vector are in the task-local
run receipt `orchestration/tasks/harness109-delivery/advertised-variant-oracle-run-receipt-03f0422.json`;
the adjacent `.log` stores console output and exit status, not the invocation.

The observed result was 1 passed, 0 failed. Its five probes were `describe`, `describe_repeated`,
`map_refused_pre_inference`, `management_refused_pre_inference`, and `tampered_config_rejected`;
the loopback model observed zero requests. The report binds bridge SHA-256
`97985d0d77cb0ab70765d7f176a99ec4c442fac1a314d5784ed0f98c9cb29e70` and records
`full_runtime_admission: false`.

## Artifact provenance boundary

The manifest is embedded in the Harness executable through `include_bytes!` and
`include_str!`. Updating these evidence pointers and run digests changes the bytes of a subsequent
bridge build. The Cargo build portion of the post-refresh, one-job, owned-target run was:

```sh
cargo build --locked --config 'profile.dev.package.sha2.opt-level=3' \
  --package sts2-harness --bin sts2-exo-bridge
```

It produced bridge SHA-256
`da44b1d7d40282b9486138885921767ab1b816c074731718c7ae2e4a1fa1ce64`, which differs from the
`97985d0d...` bridge recorded by both process runs. This was a build, not another process-oracle
run. Each JSON report and its manifest entry therefore retain the bridge digest and harness source
revision observed for that run. The metadata refresh does not attribute either old process run to
the rebuilt binary; no process-oracle result for the rebuilt binary is claimed here.
