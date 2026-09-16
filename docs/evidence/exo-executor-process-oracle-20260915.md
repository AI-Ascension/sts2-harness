# Evidence: real pinned-Exo one-shot executor process oracle (2026-09-15)

Issue: AI-Ascension/sts2-harness#139. Contract: `sts2-exo-bridge-v1` / ADR 0017 / ADR 0018.
Machine-readable record: [`exo-executor-process-oracle-20260915.json`](exo-executor-process-oracle-20260915.json).

Status: reproduced real-process evidence. It replaces the extension identity carried by
[`exo-extension-real-spike-20260914.md`](exo-extension-real-spike-20260914.md), which describes
extension bytes that are no longer the shipped bytes (see **Superseded identity** below).
A real provider, a native game instance, gameplay settlement, and a deployed package remain
`unverified`.

## Path under test

The oracle exercises the ADR 0018 production boundary: the harness `sts2-exo-bridge` binary reads
one strict `sts2.exo-bridge-wire-v1` request, calls the separately built `sts2-exo-executor`
embedding package, and the package runs the pinned real Exo TypeScript runtime with the owned,
tool-free extension at `experiments/exo-agent/extension/src/index.ts`. Model binding is `o3-pro`
so the frozen `ResponsesRuntime` path is selected. The model endpoint is an original synthetic
loopback server; no provider, credential, network egress, game, save, or native instance is used.

## Identities

| Field | Value |
|---|---|
| `exo_revision` | `b06869ab789dee3f80ca474b5fa89dbe47ccb859` |
| `extension_sha256` | `2e5485127f434bdd95a534785a414fa9f357432c924d89fd56d50c96f434b9cd` |
| `oracle_sha256` | `c9cfedd7f4541283a804c002d566992db7c6abdc49db554a5f414c117cce763e` |
| `executor_sha256` | `c739ff69d16250e31640e00d7611ff514c706a68c2c6bb9d332d096c713627fd` |
| `bridge_sha256` | `d7db74112df92c4c0691096f69b9cf114249435a7ccf48ae71217b89e0b81b74` |
| `harness_revision` | `aee12600150f020436bd8e6f471f573a34b56930` |
| Node | `v22.14.0` (the extension pin; upstream declares `22.15.0`, still unqualified) |
| Rust toolchain | `1.97.1`; `aarch64`/non-Linux platforms remain unverified |
| Model binding | `o3-pro` → synthetic loopback endpoint, model route asserted, no credential |

`extension_sha256` is the shipped extension file; `oracle_sha256` is the oracle test source. The
`executor_sha256` and `bridge_sha256` are locally built executables, not publication artifacts.

## Result

All 27 oracle cases passed (`test result: ok. 1 passed; 0 failed`), the whole matrix in 24.88 s:

| Case group | Cases | Observed |
|---|---|---|
| Accepted terminal decisions | `action`, `plan`, `wait`, `reobserve` | exactly 1 forwarded model request each, 0 denied, correlated `exo_session_id`/`exo_turn_id`, `argv_private_values_absent: true` |
| Model-output rejection | `illegal_action`, `multiple_json`, `truncated_json`, `empty_output`, `oversized_output`, `unknown_field`, `refusal`, `tool_escalation`, `multiple_messages` | 1 forwarded request each, then bounded rejection with non-zero executor status |
| SDK retry containment | `429_one_egress`, `500_one_egress` | 3 fetch attempts, 1 forwarded, 2 denied locally; exactly one egress |
| Pre-model rejection | `describe`, `wrong_revision`, `unsupported_map`, `wrong_generation`, `unknown_field_input`, `unsupported_expert`, `invalid_utf8`, `oversized_input`, `duplicate_field_input`, `missing_eof`, `package_digest_mismatch` | 0 model requests, non-zero exit where applicable |
| Process boundary | `executor_nonzero` | rejected before any model call |

`--describe` reports `full_runtime_admission: false`, so no case claims full episode admission.
Request-level rejection, identity, and envelope checks happen before the model is contacted, which
is the source/process half of acceptance criterion 3. Admission for a dispatched episode still
belongs to the production transport issue (#141).

The matrix above was run three times — at harness revisions `1aaa3f3`, `3d2f1ac` and `aee1260` — and
every run agreed on the case names, the pass/fail outcome, the per-case request counts, and the
`extension_sha256`/`oracle_sha256`/`exo_revision` identities. After normalising the two runtime-generated
per-case identifiers, the reports are byte-identical apart from `bridge_sha256`, which tracks the
rebuilt harness binary, and `harness_revision`. The committed record is the last run, so its
`harness_revision` is the revision the harness was at when it ran.

## What this establishes

The real pinned Exo runtime, reached through the owned one-shot executor package, loads the current
extension bytes, produces exactly one correlated terminal decision per accepted case, contains SDK
retries to one egress, and rejects pre-model identity/schema/profile failures without contacting
the model. The record binds that result to the exact revision and artifact digests listed above.

## What this does not establish

No native game instance, package deployment, provider credential, gameplay settlement, durable
recovery, map/expert profile, context continuity, cancellation durability, or non-Linux platform.
The manifest `deployment_axes` stay `unverified` because they describe a deployed instance, which
this process oracle does not create.

## Superseded identity

`exo-extension-real-spike-20260914.md` records `extension_sha256 = 8321d446…`, which was the
extension file before commit `835509b` replaced it with the current owned, restricted-profile
executor (`2e548512…`). That record is retained as historical evidence for the loader path it
exercised; it does not describe the shipped extension. This record supersedes it for extension
identity, and the binding test below fails closed if either record's identity drifts again.

## Reproduction

Requires the pinned Exo checkout at `b06869ab789dee3f80ca474b5fa89dbe47ccb859` with an installed
`node_modules`, Node `22.14.0`, and both binaries built:

```sh
cd "$HARNESS_ROOT"
cargo build --locked --config 'profile.dev.package.sha2.opt-level=3' \
  --package sts2-harness --bin sts2-exo-bridge
CARGO_TARGET_DIR="$PWD/target/exo-executor" cargo build --locked \
  --manifest-path experiments/exo-agent/bridge/Cargo.toml
STS2_EXO_TEST_NODE="$NODE_BIN_DIR/node" STS2_EXO_TEST_SOURCE="$EXO_SOURCE_ROOT" \
  CARGO_TARGET_DIR="$PWD/target/exo-executor" cargo test --locked \
  --manifest-path experiments/exo-agent/bridge/Cargo.toml --test process_oracle -- --ignored
```

The oracle writes `target/exo-smoke-report.json`; the committed JSON record is that file,
unmodified, except that no absolute host path appears in it. It creates only loopback listeners,
disposable private state under the owned `target/` directory, and bounded redacted metadata; it
clears inherited credentials and never starts a game.

## Enforcement

`crates/harness/tests/support/exo_contract_process_evidence.rs` re-derives `extension_sha256` and
`oracle_sha256` from the repository bytes and compares them with this record and with the artifact
manifest. Any change to the extension module or to the oracle source without a refreshed
real-process run fails the workspace test suite.
