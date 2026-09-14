# Candidate-root TypeScript loader

This directory contains the owned tool-free single-turn module and its boundary tests. It is not
an npm workspace package and does not publish
`@exo/harness` or `@exo/model-runtime`: the pinned Exo revision has one root package named `exo`.
Those specifiers are TypeScript `tsconfig.json` path aliases into that root checkout. Use the
[owned bridge build/process instructions](../bridge/README.md) for the current machine entrypoint.
The historical CLI loader instructions below remain a source spike, not the production bridge.

To run the synthetic loader against the reviewed candidate, use an operator-owned checkout and
keep this repository's source tree separate:

```sh
git clone https://github.com/exoharness/exo.git "$EXO_ROOT"
git -C "$EXO_ROOT" checkout --detach b06869ab789dee3f80ca474b5fa89dbe47ccb859
corepack prepare pnpm@10.26.2 --activate
cd "$EXO_ROOT"
corepack pnpm@10.26.2 install --frozen-lockfile
mkdir -p experiments/exo-agent/extension/src
cp "$STS2_HARNESS_ROOT/experiments/exo-agent/extension/src/index.ts" \
  experiments/exo-agent/extension/src/index.ts
corepack pnpm@10.26.2 exec tsgo --noEmit -p tsconfig.json
corepack pnpm@10.26.2 exec oxlint --deny-warnings \
  experiments/exo-agent/extension/src/index.ts
corepack pnpm@10.26.2 exec vitest run
cargo build --locked --package exo
./target/debug/exo --harness typescript agent create sts2-exo \
  --module "$EXO_ROOT/experiments/exo-agent/extension/src/index.ts" \
  --model "$EXO_MODEL"
```

The commands are deliberately rooted at the candidate checkout: `pnpm install --frozen-lockfile`
uses its checked-in lockfile, while `tsgo`, `oxlint`, `vitest`, and the TypeScript loader resolve
the aliases in the candidate root `tsconfig.json`. The Rust `exo` CLI is built from the candidate
Cargo workspace; `cargo build --locked --package exo` produces `target/debug/exo`, and
`--harness typescript agent create NAME --module ABSOLUTE_MODULE_PATH --model MODEL` is the exact
module-loading command. Node `22.14.0` and pnpm `10.26.2` are pinned by the adjacent
`package.json`. The reviewed upstream candidate `mise.toml` pins `nodejs = "22.15.0"` while this
extension declares `22.14.0`; this is a documented divergence. The recorded loader spike ran under
`22.14.0`, and that record is intentionally left unchanged; no run under `22.15.0` has been
recorded. The final
agent creation is an operator spike only; it needs `EXO_MODEL` plus a
model binding/credential and does not prove an STS2 terminal decision, bridge correlation, or
gameplay effect.

The owned single-turn bridge validates immutable configuration and strict envelopes before using
this module through the real executor embedding API. `ExoAdmittedTransport` separately supplies
full-preflight envelope handoff; the single-turn package does not claim the lifecycle guarantees
required to enable full episode admission. See ADR 0018 for that distinction.

Issue #140 adds a closed, operator-side `ExoRestrictedProfile` contract in the Rust trusted
configuration (`ExoToolCatalog` plus `ExoPrivateStatePolicy`). Admission validates it before
inference, and the reviewed model tool allowlist is intentionally empty until separately admitted
read-only query adapters exist. The extension rejects agent-tool creation and non-empty
library-module configuration, registers no model tools, and guards the actual fetch boundary to
forward at most one bounded request. Attempted/forwarded/denied counts include upstream SDK retry
attempts. Reviewed OS/service containment, hard state quotas and durable recovery remain separate
gates; this module is not a sandbox for arbitrary untrusted JavaScript.
