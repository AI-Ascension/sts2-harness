# Changelog archive: 2026-10-04

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link
paths, which are corrected so they resolve from this directory; this file is a verbatim record,
not a supported release or a second normative changelog.

- **Run the workspace doctests in CI.** No gate executed doctests: the `rust` job runs
  `cargo test --workspace --all-targets --all-features --locked`, and `--all-targets` excludes the
  `--doc` target by definition, so the repository's single doctest — the `compile_fail,E0624` guard
  proving that an external caller cannot construct an `AuthenticatedWorkerRequest` — had never been
  compiled in CI. A new `Run doctests` step runs `cargo test --workspace --doc --all-features
  --locked`, so that authority-boundary assertion (and any future doctest) is now executed and
  cannot rot silently. Compatibility: CI-only; no source, schema, route or behavior change.
  Refs #479.

- **Repair nine intra-doc-link defects and gate the class durably.** `sts2-harness` failed a
  documentation-integrity expectation its own gates could not see. Seven intra-doc links across six
  files named a type or method that does not resolve at the file's own scope — three of them
  reachable by a plain `cargo doc` — and two more named a bare `[`plan`]`, ambiguous between a
  function and a module (`benchmark_manifest::branch_experiment` and `benchmark_manifest::suite`).
  Every one names a real item elsewhere in the crate, so each was a scope/path or disambiguation
  defect rather than a stale name: `execution::types::worker` looked for `ExecutionStore` under
  `execution::types`, which re-exports only `ExecutionStoreError`;
  `management::save_profile_setup::setup` attributed `verify` to `VerifiedProfileReadback` when it is
  an inherent method of `ProfileReadback`; `provider_session::types::effective_limits` linked a bare
  `ProviderSessionPolicy`; and `context_control::membership`, `context_control::model_view` and
  `management::lifecycle` linked bare names owned by sibling modules. Each link now carries a path —
  or, for the two ambiguous `plan` links, a `()` disambiguator — that resolves. The durable half is a
  `cargo doc` step in the `rust` job with
  `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links --document-private-items"`, because a default
  rustdoc run skips the private modules that hold four of the seven unresolved links; no workflow had
  run `cargo doc`/`rustdoc` before, so nothing owned the class. Refs #477.

- **Make the doctest gate's guarded property actually protected.** `#480` began executing the
  workspace's doctest, but its `compile_fail,E0624` annotation does not enforce the error code. On
  rustdoc 1.97.1 a snippet that dies of `E0432` (unresolved import) or `E0425` (undeclared name)
  still reports `ok`, and an unknown code such as `E9999` is accepted silently, so only
  "compilation fails for some reason" was ever asserted. The fence reaches the `pub(crate)`
  constructor through four public re-exports, so removing any one of them would have left CI green
  while the assertion stopped testing the constructor at all — the same class the parent gate was
  added to prevent, one level up. The doc comment now carries a second, **compiling** fence that
  pins those same paths and turns red the moment one is renamed or removed; the `compile_fail`
  fence is left to assert the authority property it can actually assert. Compatibility: docs and
  doctest only; no production code, schema, route or behavior change. Refs #481.

- **Gate Exo compatibility with the real pinned Exo process in CI.** No workflow executed the
  repository's own Exo bridge lane: `experiments/exo-agent/bridge` is `[workspace]`-excluded, so
  `cargo test --workspace` could not reach the `#[ignore]`d `process_oracle`/`lookup_oracle` tests,
  and the pinned `exoharness/exo` checkout in `ci.yml` fed only lifecycle fixtures. A new
  `exo-process-oracle.yml` installs the pinned Node/pnpm, stages the owned extension into the pinned
  read-only Exo checkout, builds the isolated `sts2-exo-executor` and `sts2-exo-bridge`, and runs both
  oracles against the real Exo TypeScript runtime with a synthetic loopback model and synthetic host.
  The `EXO_SOURCE_REVISION` pin is re-read at run time and the bounded evidence report is asserted to
  name it. This proves process composition only; live provider, game and native acceptance remain
  separate. Refs #148.
