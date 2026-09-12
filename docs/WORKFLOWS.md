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

## Runtime peer contract

`runtime-peer-contract.yml` builds the candidate harness with the exact gateway and
MCP revisions in `contracts/runtime-peer-lane.json`, then executes the real
harness → MCP → gateway process chain. The only synthetic component is a bounded
game-mod HTTP endpoint owned by the harness test fixture; it is downstream of the
real peers and has no game, provider, or host authority.

The executable test proves the fixed route/catalog sequence, distinct identities,
lease epoch forwarding, unknown-operation reconciliation, and owned-process
teardown. It deliberately sends a foreign identity envelope and a malformed
expert-state envelope; both must fail before an action is forwarded. Separate
persisted-startup and cancellation-cleanup regressions remain required in the
same lane. This is synthetic process-composition evidence, not native-host or
game-effect evidence.

The ordinary pull-request and `main` paths use the immutable default peers. A
coordinated candidate pair is permitted only through `workflow_dispatch` with
full 40-hex gateway and/or MCP revisions; the checkout HEADs are compared to
those inputs. Review the resulting positive and negative evidence before editing
the default pins. Never substitute a branch name, moving default, or dirty tree.

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
