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
