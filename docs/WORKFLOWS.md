# Development and Automation Workflows

## Change lifecycle

```text
contract or design -> focused change -> local policy/tests -> review -> authorized merge -> release candidate -> publication -> verification
```

Green CI is not approval to merge, a merge is not a release, and publication is not runtime or game
verification.

## Foundation workflows

`ci.yml` runs pinned-toolchain Rust format, Clippy, and tests. `policy.yml` tests and runs the
repository policy tool. Both use `pull_request` and pushes to `main`, explicit read-only contents
permission, bounded timeouts, cancellation only for superseded pull-request runs, and immutable action
commit pins. They do not access secrets, proprietary game files, providers, or runtime environments.

Do not create empty success jobs for future game, provider, replay, or release lanes. Add a workflow
only when its command, inputs, outputs, and evidence semantics are real and make it a required check
only after branch protection is configured externally.

## Non-Linux documentation surface

`non-linux-doc.yml` documents `sts2-harness` for `x86_64-unknown-freebsd`, the target whose `cfg` set
reaches `worker_endpoint.rs`'s `#[cfg(not(any(target_os = "linux", windows)))]` arm — the arm the host doc
gate and the `windows-check` doc step compile out. It exists to make that file's coverage 3 of 3 rather
than 2 of 3, the count an earlier repository in this org once closed incorrectly. The target must be real,
not a flag: `--cfg` cannot unset a built-in `target_os`. Of the targets whose predicate is true,
`x86_64-apple-darwin` needs a macOS C toolchain the org does not have and `wasm32-unknown-unknown` is
refused by `getrandom`, so `x86_64-unknown-freebsd` is the leg. It is a separate workflow rather than a
`ci.yml` step because `ci.yml` sits at the `workflow_preferred` budget that `repo-policy --strict`
enforces. Acceptance is read from the log — exit 0, zero `error` and zero `unknown lint` diagnostics, and
the crate's `index.html` generated — not the step conclusion.

## Studio consumer contract

`studio-contract.yml` tests the candidate harness against the immutable Studio
consumer revision recorded in its checkout step. It builds from the harness
directory to select the candidate's pinned Rust toolchain, installs the consumer's
locked Node dependencies, and runs its authenticated live-owner browser tests
against a disposable loopback service. The service uses only synthetic test
credentials and a temporary SQLite store and is stopped on exit.

This producer-side check detects owner API drift before Studio adopts a new
harness pin. It complements Studio's consumer-side test of its pinned owner.
Changes to the Studio revision require reviewed consumer regression evidence;
new incompatible APIs require a coordinated rollout, not silently moving the pin
to bypass a failure. It does not exercise a game, provider, or deployment.
Maintain the stable job name when configuring required branch checks externally.
Failed consumer runs retain their synthetic Playwright traces and error context for
seven days, keyed by candidate SHA and run attempt. The Studio consumer revision
and artifact inventory also stay aligned with the effective-limit conformance lane.

## Effective-limit consumer contract

`console-contract.yml` checks out the exact reviewed Console and Studio revisions in the
effective-limit matrix. The standalone Rust
[`consumer-conformance` tool](../tools/consumer-conformance/README.md) builds the candidate
producer with its root lockfile and compiles the unchanged Console fixture-generator source
against Cargo's exact candidate library artifacts. Candidate output must equal both consumer
goldens before their unchanged admission tests run. The tool records actual compilation
provenance separately from the golden's historical origin label.

The matrix validator recognizes only the named repository/workflow pairs. It parses YAML and
requires an immutable `actions/checkout` step with exact `with.repository`/`with.ref` values
in an unconditional static job. Shell bodies and unrelated actions cannot supply a checkout.
Conditional/dependent/matrix jobs, conditional steps, mixed run/action steps and tolerated
checkout failures cannot establish the pin. The bounded parser rejects ambiguous duplicate keys,
aliases/anchors/merges/tags and multiple documents before loading the tree; this deliberately
supports the current static lanes rather than evaluating GitHub expressions.
Aligned consumers require the same revision in their matrix and
CI pin. Wrong repositories, unknown workflow labels, stale refs and absent pins fail closed.
The four Console copied-artifact hashes and Studio adapter/fixture inventory are checked in the
actual pinned checkouts. This lane establishes synthetic producer/consumer conformance only.
The existing Studio authenticated workflow-owner regression remains independently required.

## Runtime peer contract

`runtime-peer-contract.yml` builds the candidate harness with the exact gateway and
MCP revisions in `contracts/runtime-peer-lane.json`, then executes the real
harness → MCP → gateway process chain. The only synthetic component is a bounded
game-mod HTTP endpoint owned by the harness test fixture; it is downstream of the
real peers and has no game, provider, or host authority.

The lane runs the generic runtime composition and served `serve-workflow`
policy, restart, and managed-context regressions against those same peers. The
policy case exercises run-scoped policy GET and adoption, settles one action,
and verifies the live context binding; its sibling adopts a changed policy while
the run sits at an idle decision cursor and requires the following decision to
execute rather than fence the run. The restart case persists an unknown
operation, restarts against the same stores, and proves a later step is refused
without a second effect. The managed-context case publishes and adopts an
allowlisted source against the actual decision cursor, then verifies one
provider exchange contains the retained item; its missing-source case fails
before exchange. Both managed-context cases stop before game execution. They
use the test-only local provider bridge.
The generic case deliberately sends a foreign identity envelope and a malformed
expert-state envelope; both must fail before an action is forwarded. Separate
persisted-startup and cancellation-cleanup regressions remain required in the
same lane. This is synthetic process-composition evidence, not native-host,
native-provider, or game-effect evidence.

The lane also runs both `runtime_v4_rest_executable_composition` selector cases, one per selector
encoding (`synthetic` and `native`), against the same peers. Each drives an authored
observe → decide → execute-action → terminal graph through the served REST surface and requires every
durable receipt to settle with its original operation identity and effect witness before the next
step is admitted. Each composition writes under its own evidence directory, so a REST run cannot
overwrite the generic composition's `result.json`.

The lane additionally requires the declared peers to serve the harness's own recovery sideband.
The MCP peer is started with the `watchdog-recovery-v1` profile and must advertise the exact
nine-tool sideband surface, and both peers must declare the same recovery frame contract and schema
digest the harness pins. The default MCP pin moved from `f3b6eaa8` to `587a53ce` because the older
revision predates that profile and refuses it, so the lane declared a pair its own recovery path
could never reach. The gateway pin is unchanged: it already declares the same contract and digest.

The lane also runs the shipped host-lease campaign downstream. It builds the operator-only
`synthetic_mod_server` target, points `STS2_SYNTHETIC_MOD_SERVER_BINARY` at it, and requires that
environment-configured process to advertise `host_lease=enabled` and to terminate a signed
`lease_install_request` exactly as the in-process terminal does. The in-process terminal is already
covered by an ordinary workspace test; this is the step that fails if the environment half of the
sideband is removed. It needs no gateway, MCP, or provider, and it grants no game or host authority.

The ordinary pull-request and `main` paths use the immutable default peers. A
coordinated candidate pair is permitted only through `workflow_dispatch` with
full 40-hex gateway and/or MCP revisions; the checkout HEADs are compared to
those inputs. Review the resulting positive and negative evidence before editing
the default pins. Never substitute a branch name, moving default, or dirty tree.

## Exo process oracle

`exo-process-oracle.yml` runs the repository-owned, Linux x86_64 loopback lane documented in
[`experiments/exo-agent/bridge/README.md`](../experiments/exo-agent/bridge/README.md). It installs the
pinned Node runtime and pnpm, checks out `exoharness/exo` at the revision read from
`EXO_SOURCE_REVISION`, builds the isolated `sts2-exo-executor` package and the `sts2-exo-bridge`
binary, and executes the previously `#[ignore]`d `process_oracle`, `lookup_oracle`, `fault_oracle`,
`bound_oracle` and `advertised_variant_oracle` tests. `bootstrap_oracle` is the one remaining
manual bridge oracle: it writes no report, so there is no artifact for the lane to assert.
Each executed leg names its single test (`-- --ignored --exact <name>`) and runs through
`tools/exact-gate.sh`, so a renamed, dropped, or un-ignored test fails the step instead of reporting
a green empty run (`sts2-harness#535`). The covering check is
`crates/harness/tests/exact_gate_lane_coverage.rs`, which sweeps every workflow file rather than an
enumerated few (`sts2-harness#536`).

The gate's own accounting is covered by `tools/exact-gate-selftest.sh`, which `policy.yml` runs on
every PR. It counts *executions* rather than passes, so a test that ran and failed is reported as
`executed=1` alongside the command's own failure, not as `0 executed` with a
renamed-or-removed diagnosis that would send a reader after the wrong cause
(`sts2-harness#540`). One invocation naming several `--exact` filters counts one execution per
test, so batching filters is satisfiable.

The gate reads the guarded command's output, so it guards against a *renamed, dropped or
un-ignored* test, not against a test that prints libtest's own summary lines itself. That limit is
pre-existing and is recorded in the script header.

`fault_oracle` adds the `#148` T2/T3 fault and isolation matrix: the admission faults (config schema,
pin and argv identity, provider-route refusal) must fail closed with **zero** model egress, a lost
model reply must fail closed within the bounded process lifetime, and two sequential or concurrent
runs must each send exactly one model request from a private endpoint, config directory and
`TMPDIR`. State outside `TMPDIR` (the shared binaries, cargo target and Node install) is deliberately
not asserted disjoint, and no barrier forces the concurrent runs to overlap. It emits the bounded
`target/exo-fault-report.json`, whose `exo_revision` is asserted against the runtime pin so a moved
pin cannot leave a stale report.

`bound_oracle` adds the two source-only remainders `#148` recorded after those slices: writer-side
back-pressure and the executor's turn budget. A writer offering 131,072 and 131,073 bytes gets the
bridge's own pre-inference parse refusal with **zero** model connections, and the report records the
byte count the writer got in rather than asserting it exactly, because it is partly a kernel pipe
property. The executor's 160 KiB read bound is then pinned by handoffs padded to the bound, one byte
past it, and 4 KiB below it: the middle size is *admitted* by the read bound but the turn is denied
locally by the extension's equal model-write bound, so the read bound is reported as reachable only
by a direct drive rather than as a size that yields a decision. A saturated bridge projection (32
constraints of 512 bytes) is measured below the executor bound, and the reason the read bound is
unreachable is the bridge's own 131,072-byte parse bound, 32 KiB under the read bound, plus a few
hundred bytes of handoff framing — not the constraint cap the case happens to saturate.
The budget half drives the executor against an endpoint that holds the reply outstanding, so the
typed `exo_executor_turn_timeout` is measured against a real deadline, and against a reply the
provider truncated at `max_output_tokens`, so no decision is fabricated.
`target/exo-bound-report.json` carries the same asserted `exo_revision` and
`full_runtime_admission: false` as the sibling reports.

The oracle drives the real pinned Exo TypeScript runtime. Only the model service is replaced with a
bounded synthetic loopback endpoint and the native game with the declared synthetic host; no
provider, credential, game, save or native instance is used. The lane proves real Exo **process
composition**, not live model or game compatibility, and it stays separate from the live-provider and
native-host acceptance in the Exo integration tracker. The bridge package is `[workspace]`-excluded,
so this is the only gate that reaches it.

## Authoring rules

- Keep each workflow focused and under 200 nonblank lines, preferably under 160.
- Pin third-party actions to a full commit SHA and retain a version comment.
- Start with top-level `permissions: contents: read`; elevate only through a reviewed job-specific need.
- Set explicit job timeouts and bounded concurrency.
- Use read-only `pull_request` for untrusted changes; never use `pull_request_target` here.
- Do not use `continue-on-error: true`, `|| true`, blanket retries, or hidden skips.
- Do not upload prompts, model output, saves, credentials, personal paths, or unsanitized diagnostics.
- Keep local commands equivalent to CI and report unavailable lanes as unverified.

## Branch and review flow

Keep one cohesive responsibility per change. Inspect and preserve unrelated dirty files. Changes to
public records, lifecycle, ports, provider security, retention, dependency direction, protocol scope,
or release artifacts require an ADR or explicit design review. Pull requests state the exact
commands/results, compatibility classification, evidence level, data/security impact, and remaining
limitations.

## Release and runtime authority

Release, provider, game launch, profile mutation, deployment, publication, and tag operations need
separate explicit authorization. No workflow may download or redistribute proprietary game files or
use fork-controlled code with secrets. Runtime lanes use authorized disposable environments and record
exact versions, artifact digests, cleanup, and visible skips.

## Current validation

Run:

```bash
cargo run --locked --package repo-policy -- --strict
```

Then run the Rust checks in [`TESTING.md`](TESTING.md). Workflow lint/security tooling may be added
later; until then, policy pin checks and human review remain the available workflow evidence.
