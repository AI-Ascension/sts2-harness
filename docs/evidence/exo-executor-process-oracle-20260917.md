# Evidence: real pinned-Exo one-shot executor process oracle (2026-09-17)

Issue: AI-Ascension/sts2-harness#140. Contract: `sts2-exo-bridge-v1` / ADR 0017 / ADR 0018.
Machine-readable record: [`exo-executor-process-oracle-20260917.json`](exo-executor-process-oracle-20260917.json).

Status: reproduced real-process evidence. It adds the by-name forbidden-tool denial cases and the
empty-tool-request case for issue #140 and supersedes
[`exo-executor-process-oracle-20260915.md`](exo-executor-process-oracle-20260915.md) for extension
identity: the shipped extension now seals the actual tool registry, so its bytes changed. The
earlier record is retained as historical evidence for the extension it exercised. A real provider, a
native game instance, gameplay settlement, and a deployed package remain `unverified`.

## Path under test

The oracle exercises the ADR 0018 production boundary: the harness `sts2-exo-bridge` binary reads
one strict `sts2.exo-bridge-wire-v1` request, calls the separately built `sts2-exo-executor`
embedding package, and the package runs the pinned real Exo TypeScript runtime with the owned,
tool-free extension at `experiments/exo-agent/extension/src/index.ts`. Model binding is `o3-pro`
so the frozen `ResponsesRuntime` path is selected. The model endpoint is an original synthetic
loopback server; no provider, credential, network egress, game, save, or native instance is used.

The forbidden-tool cases drive the synthetic model to emit a `function_call` whose `name` is a
forbidden upstream tool, alias, or case/namespace variant. Because the reviewed catalog is empty and
the extension seals the actual `HarnessToolRegistry`, dispatch throws the typed `sts2_forbidden_tool`
error before any handler exists. Each case is checked twice: once through the whole
bridge (fail-closed `exo_bridge_executor_failed` after exactly one model egress) and once directly
against the executor, whose receipt carries `error_code: exo_forbidden_tool` and no decision.

## Identities

| Field | Value |
|---|---|
| `exo_revision` | `b06869ab789dee3f80ca474b5fa89dbe47ccb859` |
| `extension_sha256` | `bcc034e787972f7ad6eabff5e817ad1456d6f6cab8c7dc42b426bd9f5b33ef3d` |
| `oracle_sha256` | `381ba1470f476872bded864400890ce6bbcb63d256496d76729093ab080114eb` |
| `support_sha256` (`tests/support/mod.rs`) | `c42408176a7f774d1f845cb3b8a0805892174ae237149ca037d232bf65050b5c` |
| `support_sha256` (`tests/support/projection.rs`) | `25c6168f0c76d103d7fc0e1ca14ac0bf8251c35fdc5cadd31ea50318a455c26e` |
| `executor_sha256` | `35b214b58d3cd5fdcf250078b6dec1fcc24b6f0bc77b58fbfdb91e103062dc70` |
| `bridge_sha256` | `d2d7e7a1e5f6185b0d5d141c102fddd9fb726d30f3d3dba1313ba973b443bed2` |
| `harness_revision` | `89c489649c1912bd828e9c8b2fda42376c483a11` |
| Node | `v22.14.0` (the extension pin; upstream declares `22.15.0`, still unqualified) |
| Rust toolchain | `1.97.1`; `aarch64`/non-Linux platforms remain unverified |
| Model binding | `o3-pro` → synthetic loopback endpoint, model route asserted, no credential |

`extension_sha256` is the shipped extension file; `oracle_sha256` is the oracle test source and
`support_sha256` binds the two oracle support modules. The `executor_sha256` and `bridge_sha256` are
locally built executables, not publication artifacts.

`harness_revision` names the commit at which the recorded sources were frozen, which is necessarily
an ancestor of the commit carrying this record and is rewritten when the change is squash-merged.
The binding that matters is that the named revision contains every recorded source byte, which the
oracle now enforces at record time (`support::assert_sources_are_committed`).

## Result

All 40 oracle cases passed (`test result: ok. 1 passed; 0 failed`), the whole matrix in 42.65 s:

| Case group | Cases | Observed |
|---|---|---|
| Accepted terminal decisions | `action`, `plan`, `wait`, `reobserve` | exactly 1 forwarded model request each, 0 denied, correlated `exo_session_id`/`exo_turn_id`, `argv_private_values_absent: true` |
| Model-output rejection | `illegal_action`, `multiple_json`, `truncated_json`, `empty_output`, `oversized_output`, `unknown_field`, `refusal`, `tool_escalation`, `multiple_messages` | 1 forwarded request each, then bounded rejection with non-zero executor status |
| Empty tool request | `request_tools_are_empty` | request body advertises no `tools` and no `tool_choice`; 1 forwarded request |
| Forbidden tool by name | `forbidden_tool_by_name_{shell, shell_upper, shell_mixed, shell_functions_namespace, shell_builtin_namespace, install_agent_tool, uninstall_agent_tool, manage_tool, inspect_tools, install_skill, remember, lookup_query}` | model calls the tool by that exact name; bridge fails closed `exo_bridge_executor_failed` and executor receipt is `error_code: exo_forbidden_tool` with no decision; 1 forwarded egress per boundary (2 per case) |
| SDK retry containment | `429_one_egress`, `500_one_egress` | 3 fetch attempts, 1 forwarded, 2 denied locally; exactly one egress |
| Pre-model rejection | `describe`, `wrong_revision`, `unsupported_map`, `wrong_generation`, `unknown_field_input`, `unsupported_expert`, `invalid_utf8`, `oversized_input`, `duplicate_field_input`, `missing_eof`, `package_digest_mismatch` | 0 model requests, non-zero exit where applicable |
| Process boundary | `executor_nonzero` | rejected before any model call |

`--describe` reports `full_runtime_admission: false`, so no case claims full episode admission.
The forbidden-tool cases satisfy issue #140 acceptance criterion 1: the tests invoke forbidden tool
names, aliases, and case/namespace variants against the live registry and dispatch, and none
reaches a shell, arbitrary networking, secret/store administration, raw host access, or game
mutation.

## What this establishes

The real pinned Exo runtime, reached through the owned one-shot executor package, loads the current
extension bytes, advertises no tool, denies every forbidden tool name/alias/variant at dispatch with
a typed error and no reachable handler, produces exactly one correlated terminal decision per
accepted case, contains SDK retries to one egress, and rejects pre-model identity/schema/profile
failures without contacting the model. The record binds that result to the exact revision and
artifact digests listed above.

## What this does not establish

No native game instance, package deployment, provider credential, gameplay settlement, durable
recovery, map/expert profile, context continuity, cancellation durability, or non-Linux platform.
The manifest `deployment_axes` stay `unverified` because they describe a deployed instance, which
this process oracle does not create. Materializing the declared private-state roots, quota
enforcement, and retention sweeps remain open under #140.

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

`crates/harness/tests/support/exo_contract_process_evidence.rs` re-derives `extension_sha256`,
`oracle_sha256`, and the two `support_sha256` values from the repository bytes and compares them
with this record and with the artifact manifest, and asserts the forbidden-tool cases carry the
typed executor and bridge error codes. Any change to the extension module or to the oracle sources
without a refreshed real-process run fails the workspace test suite.

The oracle itself also refuses to emit a record whose `harness_revision` does not describe the
recorded bytes: `support::assert_sources_are_committed` fails the run when any recorded source
differs from `HEAD` or is untracked, so a run against a dirty tree cannot produce a record that
names a revision lacking the evidence it claims. Both oracle reports are gated this way.
