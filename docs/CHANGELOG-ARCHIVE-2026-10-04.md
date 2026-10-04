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

- **A management response that could not be transmitted is no longer discarded in silence, and the
  read and write phases no longer share one deadline budget.** `handle_connection` took a single
  `Instant` before reading the request and passed that same, already-spent value to both
  `read_request` and `write_response`. Because `write_with_deadline` refuses an expired budget
  *before* attempting any syscall, an overran read left the response unable to be sent at all: the
  owner had computed an authoritative answer, then closed the socket without transmitting it, and
  the peer observed `ECONNRESET`/`socket hang up`. Consumers that proxy this service saw a transport
  fault for a request that had in fact been answered -- the mechanism behind Studio's intermittent
  `submission_refused_502`, which had previously been attributed in turn to an owner crash, a
  restart, a listener-readiness race, and an intermediary timeout, none of which the evidence
  supported. Each phase now derives its own budget from `HttpLimits::deadline`, so exhausting the
  read no longer consumes the write, while the bound itself is unchanged and an exhausted *write*
  budget still refuses before touching the socket. The worker no longer drops the connection result
  via `let _ =`; a connection that ends without delivering a response is reported on stderr, since
  the peer can no longer be told and that silence was what made the failure unattributable. Refs
  #816, ascension-workflow-studio#214.

- **The restricted Exo profile now enforces its tool catalog at dispatch, and the enforcement
  distinguishes an unreviewed tool from an unadmitted one.** #140's merged contract validated the
  catalog as a *declaration* and stopped there: `ExoToolCatalog::reviewed()` returns an explicitly
  empty allowlist, but nothing read what the model actually asked for when its answer came back, so
  an empty catalog that validated could still be a catalog nothing had ever enforced. A guard now
  reads the model's own output bytes and refuses every tool call the run's catalog does not carry,
  before `parse_decision` flattens anything into a generic malformed-response code. Names are
  resolved through their aliases first — `functions.shell`, `mcp__terminal__shell` and bare `shell`
  are one tool reached three ways, and a bare-name check is exactly the kind of thing that looks
  complete until someone addresses a tool the other way. A prompt that merely *mentions* `shell` is
  unaffected, because description is not authority. `ExoConfig::tool_catalog` defaults to the
  reviewed (empty) catalog, so the posture is fail-closed without any configuration, and the
  refusal reason distinguishes a name that is never allowed from one that is reviewed elsewhere but
  absent from this run's catalog. The walk is depth- and count-bounded so a hostile response cannot
  make the boundary check itself unbounded. Two design errors were caught by mutation rather than by
  review: a test asserting an alias of an *admitted* tool is refused was simply wrong (it fails, and
  the guard is right), and a second assumed the dispatch check and `ExoToolCatalog::validate` consult
  the same allowlist when they answer different questions — collapsing them would have duplicated a
  check a caller can bypass by skipping validation. Compatibility: additive; the reviewed allowlist
  is still empty, so no admitted run gains a capability. Refs #140.

- **ADR 0079 named the wrong root cause for the 17 runtime-binary failures, and one of the plan
  dispatch guards was untested.** Independent review of #812 / #319 found both. The ADR attributed
  the failures to an `O_TMPFILE` temporary that "cannot be created on this container's filesystem".
  Measured, that is not what fails: the unnamed temporary is created, written, and is a valid
  descriptor. The failing call is `linkat(&temporary, "", directory, name, AT_EMPTY_PATH)`
  returning `ENOENT`, which `io_error` maps to the `Missing` those tests report. Linking the same
  content by *named* path in the same directory succeeds, isolating the fault to `AT_EMPTY_PATH`
  rather than to permissions or the filesystem, and the condition is that the container holds no
  capabilities at all (`CapEff: 0000000000000000`), so it lacks `CAP_DAC_READ_SEARCH`. Two
  consequences are now recorded: the confined write path is sound wherever that capability exists
  and degrades to a typed `Missing` rather than a panic or a partial artifact, and the store does
  not currently fall back to a weaker publication path — whether it should degrade to an explicit
  unsupported-capability result is left to `ExactArtifactStore`'s owner. The durable record had
  pointed a future maintainer at a filesystem that cannot hold temporaries, when the constraint is
  a missing Linux capability.

  Separately, `ActionPlan::next` refuses to dispatch a planned step matching more than one legal
  action, because two ids carrying an identical payload cannot be told apart — a guard a refactor
  could have deleted silently. The first test written for it was vacuous and mutation testing
  caught it: it duplicated a payload in the *initial* catalog, where `ActionPlan::new` resolves by
  id and never reaches the guard. The guard only fires against the *successor* catalog, after
  `action_completed(true)` has settled the first step, because that is where the plan re-resolves a
  payload against a catalog it did not choose. The rewritten test drives that path, with a control
  asserting the same successor still dispatches when its payloads are distinct. Deleting the guard
  now fails exactly one test. No production behaviour changes. Refs #813, see ADR 0079.
