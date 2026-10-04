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
