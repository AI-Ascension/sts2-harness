# ADR 0018: Owned single-turn Exo executor package

## Status and scope

Accepted for the source/process increment of harness issue #141. The integration owner explicitly
approved an original adapter package at `experiments/exo-agent/bridge/`, embedding the reviewed
Exo executor without adding an Exo implementation dependency to `crates/harness`.

This supersedes ADR 0017's requirement that an operator supply the missing bridge executable from
outside this repository. It does not weaken that ADR's full-runtime preflight, approve a human CLI
fallback, or claim the lifecycle, episode, provider, game, replay or release gates are complete.

## Inspected machine interface

At Exo `b06869ab789dee3f80ca474b5fa89dbe47ccb859`:

- `crates/executor/src/typescript.rs` exposes `TypeScriptHarness::new`, with a source workspace,
  an `ExoHarness` handle and explicit tool runtime. Its runner loads the selected TypeScript module.
- `crates/executor/src/harness_types.rs` exposes `Harness::create_agent`,
  `HarnessAgent::create_conversation` and `HarnessConversation::send(SendRequest)`.
- `crates/executor/src/executor_types.rs::SendResult` carries independently allocated session and
  turn IDs. The conversation event port supports bounded, ordered queries.
- `exoharness/typescript/model-runtime/turn-loop.ts::runResponsesHarnessTurn` resolves the
  model binding and executes the upstream Responses runtime. An explicit empty `registerTools`
  callback avoids default, installed, library and agent-created tool registration.

These are source-inspected public APIs used by the original embedding executable. No upstream
implementation is copied. The implementation does not invoke `conversation send`, `exo-cli`,
`/health`, `/request`, a shell, an alternate provider SDK, or any game/host interface.

## Package and dependency ownership

The public entrypoint is `sts2-exo-bridge`, built by the harness workspace. It owns immutable
configuration admission, packaged-byte checks, strict request validation, private process
supervision, terminal parsing and correlated output. It has no Exo implementation dependency.

The separate `sts2-exo-executor` package is built with its own manifest and lockfile at
`experiments/exo-agent/bridge/`. It embeds exact-pinned `executor` and `exoharness` crates, using
their public APIs to run the owned TypeScript extension. Its placement is a dependency boundary,
not a claim that runtime code is merely a fixture. The package's Rust sources are production
adapter sources subject to normal policy, format, lint, test and review requirements.

The isolated lockfile was seeded from the reviewed upstream lock and resolved for this package.
In particular, upstream requires its locked `keyring 4.0.0-rc.3`; resolving its broad requirement
to `4.2.0` breaks the inspected API. Keep the locked transitive graph instead of silently using
newer dependencies. Node `22.14.0` and pnpm `10.26.2` match the previously recorded loader lane.
Upstream's Node `22.15.0` declaration remains a separate, unverified compatibility row.

## Single-turn admission and honest capabilities

The closed `sts2.exo-one-shot-config-v1` configuration supplies source/module/executable paths,
exact executor/extension/Node digests, model and endpoint. Run commands also require the SHA-256
of the exact configuration bytes. The bridge verifies the source commit and absence of tracked
source modifications, and requires the exact extension bytes embedded at bridge build time.
Installed source and binaries are trusted, operator-owned immutable inputs; this does not claim
OS-level protection against a hostile local administrator changing them during execution.

`--describe CONFIG` is non-inferencing and returns `sts2.exo-one-shot-capability-v1`, packaged
digests, the actual configured model/endpoint and `full_runtime_admission: false`. It is
deliberately not a fabricated `sts2.exo-capability-v1` full-runtime descriptor.

`--run CONFIG CONFIG_SHA256` requires the reviewed OpenAI HTTPS route and an explicitly supplied
`STS2_EXO_MODEL_KEY`. `--synthetic CONFIG CONFIG_SHA256` accepts only `o3-pro` and a literal
loopback HTTP port; it uses an original synthetic key and cannot inherit a real credential or
silently switch to an external model. The synthetic route is recorded honestly rather than
represented as `api.openai.com`.

The source increment supports one fresh **standard** request and `action`, `plan`, `wait`, and
`reobserve`. Map, expert, management/recovery, continuity and automatic plan execution are rejected.
The unchanged strict request validator checks the complete observation, catalog, objective,
constraints, revision and state/generation bindings. All admitted catalog entries and constraints
reach Exo unchanged. The original extension requests a bounded terminal JSON decision; the Rust
parser independently rejects invalid IDs, extra fields, duplicate/trailing output or multiple
terminal messages. There is no default move or repaired decision.

## Private process and network boundaries

The harness bridge writes one bounded private internal handoff to the embedding executable's
stdin, followed by EOF. The handoff separates model-visible input from host correlation,
configuration and credentials. Neither observations nor credentials enter argv. A fresh private
0700 directory contains state, configuration, cache and temporary children. The credential is
stored only in the disposable Exo store encrypted with a newly generated in-memory key.
Child environment inheritance is cleared; pricing/telemetry defaults cannot silently become
additional provider routes. The extension/source roots remain immutable inputs.

The selected upstream SDK defaults to internal retries and offers no retry option through the
reviewed `ResponsesRuntimeOptions`. The original extension therefore guards the actual fetch
boundary: it forwards at most one POST to the configured `/responses` route, refuses redirects,
bounds request/response bytes, and rejects additional fetch attempts before egress. It records
attempted, forwarded and denied counts in one custom Exo event. This does **not** claim the SDK
retry policy was disabled. An SDK retry attempt remains a failed turn, not a successful retry.

The executor receipt binds actual Exo turn/session IDs to the host request/turn pair. Only
validated bounded metadata reaches stderr; stdout contains exactly one closed decision envelope.
Model output and upstream exception text are never forwarded as diagnostics. Provider usage and
cost interpretation remain #144's separate work.

Local timeout cleanup targets the owned process group before reaping its leader. Remote model
cancellation, hard storage quotas, restart recovery and accepted/unknown reconciliation remain
unverified. Abrupt caller/bridge termination and cleanup after that termination are also outside
this lane; an outer transport must not advertise durable cancellation from its direct-child exit.
Source/process success is not a durable or native completion claim.

## Integration gates retained

`ExoAdmittedTransport` supplies the full-preflight, correlated envelope adapter over an existing
`ExoTransport`, with one explicitly bound execution/request/turn tuple and no retry. Its recording
tests prove admission and parsing, not a live provider. The single-turn package does not advertise
the cancellation/recovery/idempotency guarantees that full preflight requires, so it cannot
silently enable the full episode path.

- #142 owns durable invocation, cancellation and restart recovery.
- #143/#144 own exact prepared-input continuity and truthful usage/attempt accounting.
- #145 owns admitted CLI/runtime episodes, durable recording, resume and replay.
- #146/#147/#148 own workflow composition, release packaging and the aggregate pinned-peer CI lane.
- #149 retains actual provider/game/terminal/replay evidence and its separate authorization.

The source-local process oracle runs the built bridge and **real pinned Exo**, replacing only the
model endpoint with an original bounded loopback fixture. It verifies actual request counts,
actual Exo turn identities, complete input projection, argv privacy, strict rejection and denied
SDK retry attempts. Commands and support exclusions are in the bridge README. No source-only
or synthetic result closes those downstream gates.
