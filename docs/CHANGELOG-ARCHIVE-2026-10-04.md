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

- **Split the synthetic management adapter so it stops hiding a 531-line hard-limit breach behind a
  false exemption.** `management/workflow_ports.rs` measured **931** nonblank lines against a
  `rust_max` of **400** — more than double — and its `policy.toml` exemption claimed the file
  "remain[s] below the hard limit". Because `size_findings` skips exempt paths *before* reading
  them, the stated count is the only assertion of that count anywhere, so nothing prompted anyone
  to check it and the breach stayed green. The file is now nine modules, largest **293** nonblank,
  all inside the 400-line hard limit, and the exemption is **deleted** rather than reworded. The
  modules group along their own seams — definition admission, context inspection, capability
  reporting, in-memory and persistent execution, replay — plus a small shared support module for
  error translation, with the three fixture constructors left in `workflow_ports.rs`. The split is
  behaviour-preserving: every one of the 931 original nonblank body lines survives, modulo the
  deliberate `pub(super)` visibility the module boundaries require and the re-wrapping of the
  original single `use` block into eight per-module ones. Splitting a file is exactly the kind of
  change that silently drops an import or a visibility edge, so the split was verified by compiling
  rather than by inspection: `cargo check -p sts2-harness --lib` passes with **0 errors and 0
  warnings**, and `repo-policy --strict` reports **0 warnings, 0 errors**. Refs #570.

- **Split the provider-session policy HTTP suite so it stops hiding a 114-line hard-limit breach
  behind a false exemption count.** `provider_session_policy_http.rs` measured **714** nonblank
  lines against a `rust_test_max` of **600**, and its `policy.toml` exemption claimed **557** and
  that the file "remain[s] below the 600-line test hard limit". The size gate cannot catch this on
  its own: `size_findings` skips exempt paths *before* reading them, so an exemption's stated count
  is the only assertion of that count anywhere. Because the prose reads as a durable, reviewed
  justification, nothing prompted anyone to check it, and the breach stayed green. The suite is
  now three files — the redacted read/reopen projection, adoption identity and admitted-run
  binding, and the command lifecycle covering CAS, idempotency, restart and control grants — over a
  shared `support/` fixture module, at **133 / 171 / 288** and **171** nonblank lines, all inside
  the 400-line preferred budget, and the exemption is **deleted** rather than reworded: correcting
  the count instead of splitting the file is the exact failure mode this entry describes. All ten
  original tests are preserved and still pass, unchanged in behaviour; this is a source layout
  change only, with no production, protocol, or runtime effect. The one-string fix from the same
  family is included: `served_gateway_evidence_naming.rs` named a graph lane `graph-original`
  where the real request ids are `graph-changed` and `graph-base`, so the fixture was pinned to an
  id that does not exist — harmless today because that test only asserts pairwise distinctness,
  and silently rot for the same reason this exemption did. Refs #564.

- **Extend the real pinned-Exo CI lane with the `#148` fault and isolation matrix.** The landed lane
  executed the real Exo process oracle but exercised only a few admission rejections. A new
  `fault_oracle` test proves the admission faults fail closed with zero model egress (config schema,
  extension/node/executor pins, relative executor path, argv config identity, provider-route
  refusal), that a model endpoint which consumes the request and then closes with no reply fails the
  run within the bounded process lifetime, and that two sequential or concurrent runs each send
  exactly one model request with no shared endpoint, temporary or state root (`#148` T3). The lane
  writes a bounded `target/exo-fault-report.json` and asserts its revision against
  `EXO_SOURCE_REVISION`. This is real-process evidence with a synthetic model and synthetic host;
  live provider, game and native acceptance remain separate. Refs #148.

- **Pin the authenticated-request constructor so the guard cannot silently stop naming it.** The
  fence pair added for `#481` cannot detect a *rename* of `from_transport`: renaming it while it
  stays `pub(crate)` leaves both fences green — the `compile_fail` snippet now dies of `E0599`,
  which the inert `,E0624` clause ignores, and the compiling companion pins only the type paths and
  `WorkerCapability::Dispatch`. Neither doc fence can close this by construction, because both
  compile as an *external* crate and can never name a `pub(crate)` item. An in-crate
  `#[cfg(test)]` assertion now pins the constructor's name and signature at its `pub(crate)` path;
  it runs under the existing `Run Rust tests` target (`cargo test --lib`/`--all-targets`), which is
  different from the `Run doctests` step, and fails to compile if the constructor is renamed or its
  signature changes. Compatibility: test-only; no production code, schema, route or behavior
  change. Refs #485.
