# Owned single-turn Exo bridge

An additive lookup mode is specified in [ADR 0022](../../../docs/decisions/0022-exo-lookup-duplex-bridge.md).
Use `sts2.exo-lookup-config-v1` with the owned `extension/src/lookup.ts` and
`--lookup-describe` or `--lookup-synthetic CONFIG CONFIG_SHA256`. The host keeps stdin
open and exchanges `sts2.exo-lookup-wire-v1` frames. Production callers use
`sts2_harness::exo_lookup_process::ExoLookupProcess` with `run_lookup_tool_loop`,
an admitted `LookupSession`, unchanged legal actions and the existing MCP port.

The legacy executor entrypoint `--lookup` advertises and emits only the closed v1
query/read profile. The explicitly selected `--lookup-bootstrap` entrypoint adds
`sts2_lookup_bootstrap`; that tool emits `sts2.exo-lookup-wire-v2-bootstrap`
Bootstrap frames and requires the correlated feedback frame to retain v2. All
other frames remain v1 in the additive profile. A v1 session rejects the new
Bootstrap payload. The shipped relay selects this profile as a pair: it launches
the executor with `--lookup-bootstrap` and `STS2_EXO_LOOKUP_BOOTSTRAP=1`; direct
executor callers must set both controls together.

The additive process oracle uses an existing read-only pinned Exo source checkout.
Its extension path is this harness checkout; generated files stay in `target/`:

```sh
STS2_EXO_TEST_NODE="$NODE_BIN_DIR/node" STS2_EXO_TEST_SOURCE="$EXO_SOURCE_ROOT" \
  CARGO_TARGET_DIR="$PWD/target/exo-executor" cargo test --locked \
  --manifest-path experiments/exo-agent/bridge/Cargo.toml --test lookup_oracle -- --ignored
```

This oracle uses a synthetic loopback model and never calls a provider or a game.

`sts2-exo-bridge` consumes a strict `sts2.exo-bridge-wire-v1` request on stdin, then calls the
separately built `sts2-exo-executor` embedding package. The latter executes the pinned real Exo
TypeScript runtime and the owned, tool-free extension. Stdout is exactly one correlated decision
envelope. No operator-authored integration code or human CLI scraping is required.

This is a standard/fresh single-turn source/process increment for #141, not full episode admission.
Map, expert, management/recovery and continuity are rejected. Full lifecycle, provider/game,
terminal-episode and replay acceptance remain open under #142–#149. See
[ADR 0018](../../../docs/decisions/0018-exo-one-shot-executor-package.md).

## Isolated build and verification

Use the repository-pinned Rust toolchain, Node `22.14.0` and pnpm `10.26.2`. Node `22.15.0` is the
upstream declaration but has not been qualified by this lane. No global installation is necessary.
The source/process lane is Linux x86_64; other environments remain unverified.
Set `NODE_BIN_DIR` to the directory containing the pinned `node` and `corepack`, and run from the
harness root:

```sh
export PATH="$NODE_BIN_DIR:$PATH"
git clone https://github.com/exoharness/exo target/exo-source
git -C target/exo-source checkout --detach b06869ab789dee3f80ca474b5fa89dbe47ccb859
corepack pnpm@10.26.2 --dir target/exo-source install --frozen-lockfile
mkdir -p target/exo-source/experiments/exo-agent/extension/src
cp experiments/exo-agent/extension/src/*.ts \
  target/exo-source/experiments/exo-agent/extension/src/
cp experiments/exo-agent/extension/package.json \
  target/exo-source/experiments/exo-agent/extension/
corepack pnpm@10.26.2 --dir target/exo-source/experiments/exo-agent/extension run check
cargo fmt --manifest-path experiments/exo-agent/bridge/Cargo.toml -- --check
CARGO_TARGET_DIR="$PWD/target/exo-executor" cargo clippy --locked \
  --manifest-path experiments/exo-agent/bridge/Cargo.toml --all-targets -- -D warnings
CARGO_TARGET_DIR="$PWD/target/exo-executor" cargo test --locked \
  --manifest-path experiments/exo-agent/bridge/Cargo.toml
CARGO_TARGET_DIR="$PWD/target/exo-executor" cargo build --locked \
  --manifest-path experiments/exo-agent/bridge/Cargo.toml
cargo build --locked --config 'profile.dev.package.sha2.opt-level=3' \
  --package sts2-harness --bin sts2-exo-bridge
STS2_EXO_TEST_NODE="$NODE_BIN_DIR/node" CARGO_TARGET_DIR="$PWD/target/exo-executor" \
  cargo test --locked --manifest-path experiments/exo-agent/bridge/Cargo.toml \
  --test process_oracle -- --ignored
STS2_EXO_TEST_NODE="$NODE_BIN_DIR/node" CARGO_TARGET_DIR="$PWD/target/exo-executor" \
  cargo test --locked --manifest-path experiments/exo-agent/bridge/Cargo.toml \
  --test bound_oracle -- --ignored
```

`tests/advertised_variant_oracle.rs` is wired into the same workflow as a fifth leg, so the
advertisement it checks is re-derived on every run instead of only being byte-pinned; its
`target/exo-advertised-report.json` is asserted for zero model requests and uploaded with its
siblings. `tests/bootstrap_oracle.rs` is **deliberately manual** (`sts2-harness#531`): it is a
round-trip test that writes no report, so it has no artifact to assert or upload, and it needs the
built relay plus the pinned Exo checkout in the same way the wired legs do.

`tests/bound_oracle.rs` measures the two source-only remainders recorded on `sts2-harness#148`
after the process/fault slices: writer-side back-pressure at the bridge request bound and the
executor's own read bound, and the `timeout_millis`/`max_output_tokens` turn budgets. It emits
`target/exo-bound-report.json` and refuses to record it unless every recorded source is committed
at `HEAD`. Its evidence class is the same real-process/synthetic-model/no-game class as the
oracles above: the model service is a loopback endpoint the test controls, and nothing here
reaches a provider, credential, game, save or native host.

The SHA-256 optimization affects build performance, not admission policy; hashing large debug
binaries without it is slow. Release builds optimize that dependency normally. The isolated
executor lockfile preserves upstream's compatible prerelease keyring dependencies; do not
regenerate it against latest unconstrained registry versions. Neither target directory is a
publication artifact or an authorized deployment.

The process fixture is original synthetic data, retained in memory except for the bounded
redacted report. It creates only loopback listeners, disposable configuration and private
runtime state. It clears inherited credentials, asserts the actual Exo model route and call
count, and records denied SDK retry attempts on 429/500. It never starts a game.
The oracle explicitly selects `target/exo-test-tmp` through the bridge's `TMPDIR`; it does not
inherit the caller's temporary directory or model environment. Production callers may select an
absolute existing temporary parent; the bridge refuses a symlink parent and creates a fresh
0700 child. Each child runtime receives only its own private temporary directory.

Run the normal harness policy, format, workspace Clippy and workspace tests separately. The
isolated executor package is intentionally outside the harness dependency graph and is not
covered by `cargo test --workspace` alone.

## Configuration and entrypoints

The closed JSON configuration contains:

| Field | Value |
| --- | --- |
| `schema` | `sts2.exo-one-shot-config-v1` |
| `executor`, `executor_sha256` | absolute built embedding executable and exact SHA-256 |
| `source_root` | clean exact-pinned Exo TypeScript checkout |
| `extension`, `extension_sha256` | absolute staged owned module and exact SHA-256 |
| `node`, `node_sha256` | absolute pinned Node executable and exact SHA-256 |
| `model`, `endpoint` | admitted model and explicit route |

The extension bytes must also match those compiled into `sts2-exo-bridge`. Keep the configuration
private and immutable. It contains paths and binding metadata, never the model credential.

```sh
target/debug/sts2-exo-bridge --describe /absolute/config.json
target/debug/sts2-exo-bridge --synthetic /absolute/config.json CONFIG_SHA256 < request.json
```

`--describe` performs no model call and reports `full_runtime_admission: false`. Its endpoint
is the actual configured endpoint. `--synthetic` accepts only `o3-pro` with
`http://127.0.0.1:PORT`, supplies a fixed synthetic key, and never falls back to a real provider.
The process oracle creates this configuration automatically.

`--describe` is also machine-checkable about which variants this build implements. It publishes
`profiles` (`["standard"]`), `profile_support` (with `map`, `management` and `expert` explicitly
`unsupported`), `context_modes` (`["fresh"]`), `decisions`, `decision_support` (with `recovery`
explicitly `unsupported`), and the two fail-closed codes `unsupported_profile_code`
(`exo_bridge_unsupported_profile`) and `unsupported_recovery_code`
(`exo_bridge_unsupported_recovery`). A caller can therefore pre-check support instead of inferring
it from a rejection. The guard walks the one axis list the advertisement is derived from, so an
enforced profile is advertised in the same step; the single exempt axis (`revision`, published as
`source_revision`) is named in code and pinned by test:
`crates/harness/tests/exo_advertised_variant_negatives.rs` and the bridge's own test module assert
the agreement, and `tests/advertised_variant_oracle.rs` re-checks it against the real process with
zero model requests.

The lookup relay is terminal on an action id only, so `--lookup-describe` and
`--lookup-bootstrap-describe` re-project the decision fields instead of inheriting the one-shot set:
they advertise `decisions: ["action_id"]` and mark `action`/`plan`/`wait`/`reobserve`/`recovery`
`unsupported`. The profile fields are shared, because both entry points enforce the same profile
guard. The relay also separates two refusals it previously conflated: a non-zero start sequence is
`exo_bridge_lookup_profile` (a protocol-ordering fault), while an unsupported profile axis is
`exo_bridge_unsupported_profile` (the shared fail-closed code).

`--run` is the separately authorized provider entrypoint. It accepts the reviewed
`https://api.openai.com/v1` route and requires `STS2_EXO_MODEL_KEY` in the bridge environment.
The key crosses to the executor only through private stdin, not child environment or argv.
Do not invoke it without applicable provider authorization and bounds.

Every run also requires the SHA-256 of the exact configuration bytes as the final argument.
Requests require EOF within five seconds and are bounded to 131072 bytes. The executor has
a bounded turn deadline (`timeout_millis`, at most 120000, measured against an endpoint that
holds the reply outstanding) and a bounded output budget (`max_output_tokens`, at most 4096, so a
truncated reply yields no decision rather than a fabricated one), an isolated 160 KiB input read,
an empty model-tool registry and one fresh conversation. A handoff at that read bound is admitted
but its turn is then denied locally by the extension's equal model-write bound, so the read bound
is not a size that returns a decision. Response envelopes obey both the existing 8192-byte
ceiling and the request's lower cap.

Only actual Exo turn/session IDs and fetch-attempt counts are emitted as bounded stderr metadata.
They do not prove provider usage, game action settlement, cancellation or durable recovery.
Private state is ephemeral and cleanup errors fail the command; remote outcome uncertainty
retains its separate lifecycle gate.
