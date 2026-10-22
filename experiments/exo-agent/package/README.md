# Immutable Exo package and preflight

`sts2-exo-package` implements the source-owned package/installation part of Harness #147. It derives
a deterministic manifest from actual operator-supplied reviewed bytes, installs a private immutable
copy, and checks it without inference. Its supported row is Linux x86_64 with Rust `1.97.1`, Node
`22.14.0`, pnpm `10.26.2`, and the Exo revision in the canonical
[bridge manifest](../../../protocol-artifact/exo-bridge-v1/manifest.json).
See [ADR 0094](../../../docs/decisions/0094-immutable-exo-package-preflight.md) for the closed formats,
bounds, filesystem assumptions and the owner-readmission boundary.

The tool never downloads source, runs an installer shell script, starts a service, requests a key,
or publishes a package. It supports the existing one-shot standard/fresh v2 configuration. Lookup
and managed/continuity profiles are not included in this package format.

## Prepare reviewed inputs

Start from the exact reviewed, clean Harness revision and independently provision the reviewed Exo
checkout, its frozen dependencies, the Node distribution and dependency caches. `EXO_SOURCE_ROOT`,
`NODE_DISTRIBUTION_ROOT` and `EXO_LICENSE_FILE` below are operator-selected existing locations.
They are private local configuration, not committed fixture paths. The old example TOML's endpoint
is not a packaged executor API; this recipe uses the actual one-shot JSON configuration.

The existing [bridge recipe](../bridge/README.md) owns the locked extension checks and real Exo
oracles. These package build commands do not fetch missing dependencies:

```sh
cargo build --locked --offline --release --package sts2-harness \
  --bin sts2-exo-bridge --bin sts2-exo-package
CARGO_TARGET_DIR="$PWD/target/exo-executor" cargo build --locked --offline --release \
  --manifest-path experiments/exo-agent/bridge/Cargo.toml
```

Create a new staging directory. Do not reuse a directory containing unrelated or private files:

```sh
mkdir -p target/exo-package/inputs
chmod 700 target/exo-package target/exo-package/inputs
cp target/release/sts2-exo-bridge target/release/sts2-exo-package target/exo-package/inputs/
cp target/exo-executor/release/sts2-exo-executor target/exo-package/inputs/
cp "$NODE_DISTRIBUTION_ROOT/bin/node" target/exo-package/inputs/node
cp experiments/exo-agent/extension/src/index.ts target/exo-package/inputs/index.ts
cp Cargo.lock target/exo-package/inputs/harness.Cargo.lock
cp experiments/exo-agent/bridge/Cargo.lock target/exo-package/inputs/executor.Cargo.lock
cp "$EXO_SOURCE_ROOT/Cargo.lock" target/exo-package/inputs/exo.Cargo.lock
cp "$EXO_SOURCE_ROOT/pnpm-lock.yaml" target/exo-package/inputs/exo.pnpm-lock.yaml
cp experiments/exo-agent/extension/package.json target/exo-package/inputs/extension.package.json
cp rust-toolchain.toml target/exo-package/inputs/rust-toolchain.toml
cp LICENSE target/exo-package/inputs/LICENSE.harness
cp "$EXO_LICENSE_FILE" target/exo-package/inputs/LICENSE.exo
cp "$NODE_DISTRIBUTION_ROOT/LICENSE" target/exo-package/inputs/LICENSE.node
cp THIRD_PARTY_NOTICES.md target/exo-package/inputs/THIRD_PARTY_NOTICES.harness
chmod 600 target/exo-package/inputs/*
chmod 700 target/exo-package/inputs/node target/exo-package/inputs/sts2-exo-*
target/release/sts2-exo-package manifest "$PWD/target/exo-package/inputs" \
  "$(git rev-parse HEAD)" "$PWD/target/exo-package/manifest.json"
```

The output is created exclusively. Review its component hashes, source declaration and license
provenance and retain its SHA-256. The same bytes and declaration yield the same manifest; this does
not claim bit-reproducible compilation or attest an executable's source merely from its filename.
The manifest explicitly records `build_provenance: operator_review_required`.

## Private settings and installation

Create a private settings file from this closed shape, replacing the source locator with the
actual reviewed absolute path. The synthetic route must be a literal loopback port and its model
must be `o3-pro`; an external endpoint is rejected in synthetic mode. No key belongs in this file.

```json
{
  "schema": "sts2.exo-install-settings-v1",
  "source_root": "/var/lib/sts2-reviewed-inputs/exo-source",
  "provider_mode": "synthetic",
  "model": "o3-pro",
  "endpoint": "http://127.0.0.1:39091",
  "quota_bytes": 1073741824,
  "retention_days": 7
}
```

Protect settings with mode `0600`. Select an absolute installation root under an existing trusted
parent outside home/system/game directories, for example a provisioned `/var/lib/sts2-packages`.
The tool may create only the final root leaf; it does not change global permissions or provision a
service account. A preexisting root must be empty/private or already owned by this install format.

```sh
target/release/sts2-exo-package install "$PWD/target/exo-package/inputs" \
  "$PWD/target/exo-package/manifest.json" REVIEWED_MANIFEST_SHA256 \
  /absolute/private/settings.json REVIEWED_SETTINGS_SHA256 /var/lib/sts2-packages
target/release/sts2-exo-package preflight /var/lib/sts2-packages INSTALLATION_ID
```

Use the exact reviewed hashes and the returned 64-hex installation ID. The private configuration is
`INSTALL_ROOT/INSTALLATION_ID/config.json`; the installed artifacts are beneath `artifacts/`, and
the independently guarded state/cache/temp roots beneath `private/`. No mutable default path is
created. The last-written receipt binds all configuration and package digests. An interrupted or
already existing destination is refused without deleting or repairing it automatically.

Preflight independently checks package bytes, owner/mode/links, configuration, actual source pin and
Cargo/pnpm locks, retained-byte bounds, Node's actual version and the actual bridge description. It
does not invoke the executor, resolve an authenticated model binding, contact gateway/MCP, check an
external model, or prove synthetic smoke. Its result reports those evidence gaps explicitly. Model
credentials continue to enter only through the existing separately authorized bridge environment;
the package command never reads or forwards them. The synthetic oracle retains its original
credential-free loopback model and no-game boundary.

## Upgrade, rollback and private state

Stage a changed candidate with another reviewed manifest/settings pair. Its ID and every immutable
artifact/configuration path differ when any input changes. An existing installation cannot be
overwritten, including if its private state contains active or unknown operations. The command
does not inspect, decrypt, migrate, settle or discard operation journals.

```sh
target/release/sts2-exo-package rollback-check /var/lib/sts2-packages \
  CURRENT_INSTALLATION_ID RETAINED_INSTALLATION_ID
```

This checks both retained packages and returns `held_for_owner_readmission` with
`changes_applied: false`. It does not switch a running service. Hold new work through the actual
Harness owner, reconcile accepted/unknown operations under their original identities and package,
and obtain a new admission before configuring another installation. Do not use a new installation
to resubmit uncertain work. Retain the matching packaging tool with the old installation when
formats or compiled source pins change; unsupported migrations/rollback remain refused.

No pruning, retention deletion, broad state reset, Exo `fresh`, service restart or cleanup command
is part of this tool. Existing private-state retention remains owned by the guarded bridge and the
lifecycle owner. A partial installation can be investigated or retired only after its actual owner
proves it is unused; its mere incompleteness does not authorize deleting adjacent state.

## Verification and remaining acceptance

Run the repository's full Rust/policy gates and the focused production-boundary suite:

```sh
cargo test --locked --package sts2-harness --bin sts2-exo-package
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

The package unit suite uses original MIT synthetic executable/license inputs, real filesystem
operations and the production installer/inspection functions. Fixtures live under the single
declared task scratch root `/tmp/ai-ascension-exo-package-147-tests` and each owned child is removed
after the test. The opaque unknown-record fixture establishes byte preservation, not lifecycle
decryption or provider settlement. Missing actual Exo source is tested as a refusal before probes.

The existing hosted real-Exo oracles establish their own process results and do not automatically
qualify a new packaged installation. Actual packaged smoke, a clean operator install with supplied
reviewed Exo inputs, complete #145 episode admission, service cutover with active/unknown work,
native/provider acceptance and supported deployment rows remain open under #147/#148/#149.
Do not publish a package/release or promote those claims from this source increment.
