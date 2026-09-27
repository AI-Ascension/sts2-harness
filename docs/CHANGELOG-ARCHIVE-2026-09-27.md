# Changelog archive: 2026-09-27

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link paths,
which are corrected so they resolve from this directory; this file is a verbatim record, not a
supported release or a second normative changelog.

### Archived from CHANGELOG.md

- **Make a policy exemption waive a size limit without suppressing the evidence.** An exemption in
  `policy.toml` used to remove its file from the size check entirely — `size_findings()` did
  `continue` *before* the comparison — so the only record that an exempted file was over budget was
  a line count written in the exemption's prose. Nothing verified that number, and the two
  independent readings of the table both found **21 of its 25 stated counts stale**, six of them
  understating a file that is genuinely over a hard maximum. The gate was green with a 931-line file
  exempted against a 400-line limit. Three changes:
  - `exemptions::stated_line_count` reads the count an exemption asserts about *its own* file
    (`its 632 nonblank lines`) and deliberately ignores the limit stated beside it (`the 700-line
    markdown hard limit`). Matching only the subject form is what makes the reading exact: a naive
    scan that also matches the limit reports 23 stale entries instead of 21. A sentence that asserts
    a count but does not parse yields a finding, never a pass.
  - `exemptions::exemption_count_findings` compares that count against the file and reports a
    disagreement, and **separates severity**: stale prose on a compliant file is a warning, while
    stale prose that *understates* a hard-limit breach is an error.
  - An exemption now **waives the limit but never the evidence**, which is the decision this change
    makes explicit. An exempt file is still counted and checked; a waived hard-limit breach is
    reported under a new `EXEMPTED` severity that is visible in the output and counted separately,
    but does not fail `--strict`. The exemption *is* the machine-checkable acknowledgement — without
    that, the only thing standing between the tree and a silently oversized file would still be a
    sentence in a TOML file, and with it as a hard failure the table could never be used at all.
  The six waived breaches are now visible on every run instead of hidden: `workflow_ports.rs` (931),
  `provider_session_policy_http.rs` (714), `render.rs` (599), `state.rs` (571), `state_store.rs`
  (457), and `membership.rs` (421). Their remediation is tracked in
  [#570](https://github.com/AI-Ascension/sts2-harness/issues/570),
  [#571](https://github.com/AI-Ascension/sts2-harness/issues/571),
  [#572](https://github.com/AI-Ascension/sts2-harness/issues/572),
  [#573](https://github.com/AI-Ascension/sts2-harness/issues/573),
  [#574](https://github.com/AI-Ascension/sts2-harness/issues/574), and
  [#564](https://github.com/AI-Ascension/sts2-harness/issues/564). No size budget was widened and
  no file was split to make a number fit. The 21 stale counts are re-derived with the tool's own
  rule (`grep -cv '^[[:space:]]*$'`, matching `check_size()`'s `line.trim().is_empty()`). Compatibility:
  the policy gate's exit status is unchanged for a conforming tree; the new `EXEMPTED` line is
  additive output.

- **Correct the repository-layout document, which had drifted from the tracked tree.** The
  `docs/REPOSITORY_LAYOUT.md` tree listing was written at Wave 2 and never revisited, so it no
  longer described the repository. It listed a top-level `tests/` directory — **no such directory
  exists**; `git ls-files tests` returns nothing, and the deterministic suites live beside the code
  they cover under `crates/harness/tests/`, which the "Planned responsibility placement" table then
  repeated as the initial home for tests and conformance. It also omitted **four** tracked
  top-level directories that carry reviewed contract, fixture, and gate material —
  `contract-artifact/` (per-capability consumer contract pins with golden vectors and `SHA256SUMS`
  digests), `contracts/` (the reviewable wire pins shared with the companion console, plus the
  effective-limit and runtime-peer-lane pins), `fixtures/` (the synthetic context-control and
  context-memory corpora the suites consume), and `.github/` (the ten workflows the required gates
  run from, plus the pull-request template) — and described `schemas/`, `conformance/`, and
  `experiments/` as "future" work when all three are populated (11, 37, and 108 tracked files
  respectively, with a README in every `experiments/` subdirectory). A reader using this document to
  find the contract pins, the memory fixtures, or the gates was told, correctly, that none of them
  exist. The listing now carries every tracked top-level directory, names the real test and fixture
  homes, and generalises the changelog-archive entry to one file per archive pass rather than
  pinning only the 2026-09-10 wave while the later dated archives sit beside it. Compatibility:
  documentation only; no code, schema, policy, bound, or digest change. Source-only: no native
  effect. Refs the `docs/REPOSITORY_LAYOUT.md` drift.

- **The revision-guard self-test now pins *which* revision resolves, and refuses to run at all
  against a workflow that defines `choose_revision` twice.** The suite added for #561 asserted
  only that a success captured *something* and that a refusal captured nothing, which left two
  ways to get past it with the suite green. Changing the helper's success path to return its
  fallback instead of the supplied revision left every case passing, so the workflow would have
  silently ignored every operator-supplied pin while checking the defaults. Adding a second,
  inert copy of the original pre-#561 helper is sharper still: bash resolves a function name to
  the **last** definition, so the step would accept `refs/heads/attacker-branch` again — the
  #561 defect returning intact — and because the extractor took the *first* match, the suite
  never saw the copy at all. Every accept case now compares the resolved value against the
  supplied one on a whole-line basis, so a value carrying a suffix cannot pass as a prefix; the
  step's two `revision=$(choose_revision …)` assignments are **extracted** from the workflow
  rather than transcribed, which makes the call-site wiring a tested property too; and a workflow
  defining the helper more than once is a hard bail naming bash's last-definition rule, because
  continuing would test whichever copy the extractor happened to pick. Two cases were added for
  the same reason — a mixed supplied/empty pair, and an explicit count of what the extractor
  finds. The #561 defect itself is still caught: reinstating the original inert body fails 12
  cases, and deleting the helper bails.

- **Drain the served gateway's streams while it runs, so a chatty gateway is no longer clipped at
  one pipe buffer.** The served compositions spawn the gateway with piped stdout/stderr and, until
  this change, read **neither** pipe until `stop()` had already SIGKILLed the process group and
  called `wait_with_output()`. A pipe holds one buffer (65,536 bytes on Linux) before its writer
  blocks, so a gateway that wrote more than that was still blocked mid-write when the group was
  killed: every byte it had not yet written was discarded with no error, no truncation notice, and
  no diagnostic — the tail of a refusal, the panic after a large log line, exactly the bytes a
  reader needs. The served path was not "unbounded" as #555 assumed; it was bounded far *below*
  what the evidence layer is meant to preserve, and the bound was invisible. Both pipes are now
  taken at spawn and drained by a single background thread for the whole life of the gateway, on
  a bounded deadline, with a stated **4 MiB per-stream and 8 MiB total** capture ceiling; bytes
  past either ceiling are drained and dropped behind a machine-readable truncation notice, so a
  clipped stream is never mistaken for a whole one. A capture that cannot start kills *and reaps*
  the child rather than leaking it, and a scenario that returns early (`?`, a failed assertion)
  drops a `GatewayProcess` that now reaps its own gateway instead of leaving it serving. A
  quiet gateway's capture is byte-for-byte unchanged, and the seven `stop()` teardown sites keep
  their current semantics (`signal() == Some(9)`, `gateway_failure_evidence` still attached to
  `gateway_stderr=`). The workflow service child is untouched and keeps its own bounded
  `stop_service()`; only the gateway capture changed, because only the gateway was previously
  read after being killed. Both ceilings are covered by unit tests, and the end-to-end test — a
  stub that floods its own stderr past the pipe buffer and *then* keeps serving — recovers the
  full 2 MiB and fails against the old read-after-SIGKILL shape. Compatibility: test-support and
  CI only; no production, protocol, or runtime behaviour changes. Refs #559.

- **Bound the served gateway evidence capture, which #548 left write-through.** #548 made every
  served failure persist the gateway's own streams, but `write_gateway_streams` wrote them
  unbounded while the gateway is spawned `Stdio::piped()` with no cap and `stop()` collects via
  `wait_with_output()` — so the volume was the child's to choose, and the one *unbounded* path in
  the lane was the failure path, whose whole purpose is to explain a red. A capture above 4 MiB
  (the bound the REST evidence writer already uses) is now persisted as a byte-exact prefix plus
  an explicit truncation marker, so a cut stream cannot be mistaken for a complete one. The
  in-band `gateway_stdout=`/`gateway_stderr=` text is deliberately **not** bounded: truncating the
  attribution would trade a disk problem for the unattributable failure #548 exists to prevent.
  The truncation notice interpolates the cut point from the bound rather than repeating it as a
  literal, so a marker that exists to be trusted cannot silently report an old cut point if the
  bound moves. The bound is covered by unit tests on the writer itself rather than by an
  end-to-end case, because the spawn path cannot deliver an oversized capture: `ready()` polls
  `try_wait` and `TcpStream::connect` without reading either pipe, so a gateway writing more than
  the kernel's 64 KiB pipe buffer blocks on the write and never reaches its `bind`. Measured with
  a `Stdio::piped()` child writing 6 MiB before binding, `stop()` collected exactly 65,536 bytes
  and the child was still alive at the readiness deadline — so the bound is defence in depth
  against a future spawn that drains its pipes (#559 tracks that capture layer), and those tests
  assert the cut, the byte-exact prefix, the marker and the notice's cut point directly. Closes
  #555.

- **Stop the served compositions discarding the gateway's stderr, and stop their error strings
  implying they carried it.** Every served composition spawns the gateway with piped
  stdout/stderr and reads both back out of `stop`, but the only consumer of those bytes was
  `write_evidence`, which no `served_*` scenario calls, and no `served_*` lane step named an
  evidence directory — so on every served path the gateway's own stream was captured and then
  dropped. The gateway is the process that names a refused request header
  (`sts2-gateway#114`), which is why #541's named refusal could never be attributed. The worse
  half of the same defect was invisible rather than absent: the `served/*` error strings
  interpolated the **workflow service's** bytes behind a bare `stderr=` label, so a reader saw a
  diagnostic that looked like gateway output and concluded the gateway had reported when it had
  not. Those sites now qualify their label
  (`service_stdout=`/`service_stderr=`), and each served scenario attaches the gateway's own
  streams to its failure through one shared helper that also persists them under
  `STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR`, so the lane's diagnostic dump step finally has bytes
  to print for a served step. The regression test drives the **real** `run_served_policy_gate`
  against a stub gateway and stub workflow service that each write a marker to their own stderr,
  so it asserts this repository's plumbing rather than whether some peer revision happens to
  speak — an assertion against the real gateway would pass vacuously whenever the peer is silent,
  the condition #541 observed. It is not `#[ignore]`d, needs no operator-built binary, and
  asserts marker presence rather than an execution count, so it cannot pass by being renamed.
  The gateway's teardown-failure branch now routes through the same helper on all eight served
  paths: previously a gateway that exited non-zero *without* being signalled was reported with
  its status alone, so the one path most likely to hold a refusal explanation still dropped it.
  That branch is unreachable from the stub gateway, which is kept alive so the scenario fails on
  the service instead, so it is covered directly against a constructed `ExitStatus` rather than
  asserted by inspection.
  Compatibility: test-support and CI only; no production, protocol, or runtime behaviour changes.
  Closes #548.

- **Stop the runtime peer contract lane accepting a peer revision that is not an immutable
  40-hex commit.** `choose_revision` validated its `workflow_dispatch` input with
  `printf '%s' "$supplied" | grep -Eq '^[0-9a-f]{40}$'`, but that guard could not fail: a
  function's exit status is the status of its last command, and the last command was an
  unconditional `printf '%s' "$supplied"`, so the helper returned 0 for **any** value and
  `refs/heads/attacker-branch` was written to `$GITHUB_OUTPUT` — both peer checkouts then
  followed an operator-supplied ref. The lane's later "Verify peer revisions" step cannot catch
  it, because it compares `git rev-parse HEAD` against the same unvalidated value, so a recorded
  "ran against gateway \`<sha>\`" claim was only as trustworthy as the dispatch input. Two bash
  rules had to be defeated together, and both were measured before the fix: `errexit` does not
  propagate into a command-substitution subshell, **and** a function's status is its last
  command's status. The guard is now the last thing that can fail, writes
  `invalid immutable peer revision: <value>` to stderr, and returns non-zero, so the step's own
  `gateway_revision=$(choose_revision ...)` assignment fails and the lane aborts before either
  peer is checked out. The empty-input fallback and a valid 40-hex commit are unchanged. The
  selftest **executes** the committed function body under `bash -e` in the production
  command-substitution shape rather than asserting on workflow text, because the unfixed text
  still contains the `grep` and only the executed exit status distinguishes the two; it
  extracts the body from the workflow so a reverted workflow is what fails. It runs in the
  `policy` gate on every PR and needs no cargo, matching the existing
  `tools/exact-gate-selftest.sh` precedent.
  Deliberately **not** changed, as owner scope: whether any valid 40-hex commit — including a
  fork- or operator-controlled one — should be accepted at all. This fixes the guard the
  workflow's own input description already promised.
  Compatibility: CI/workflow only; no production, protocol, or runtime behaviour changes.
  Closes #561.

- **Stop the durable authoring-inference journal telling the loser of a reservation race that it
  won.** `SqliteAuthoringInferenceJournal::begin` inserted with `INSERT OR IGNORE` and then read the
  row back, but the read-back tested the request digest only, so the caller that lost the insert was
  classified exactly like the winner and returned `Started`; `complete` read the row and then
  updated it under a `state = 'pending'` predicate with no transaction between the two, so a
  competitor that terminalised the row in the gap left that local stale, the guard above never
  fired, and the loser was handed `Ok` for an outcome the journal had discarded. Both halves now run
  inside `TransactionBehavior::Immediate`, and `begin` binds the insert rowcount as the
  discriminator — `inserted == 1` is `Started`, and a lost insert takes the same digest-then-terminal
  branch the pre-check already used, so a reservation race cannot return two starters. The module doc
  claimed the write and its read-back were one boundary and the file contained no transaction of any
  kind, while the sibling `inference_profile_revision_sqlite.rs` it names as its pattern already
  opened one. Two store instances over one file is the shape that matters, since the `Mutex` orders
  callers only inside one process; the new regression test drives that topology and covers the
  `begin` half, the transaction closing the `complete` half's stale read and a second case covering the
  idempotent same-state repeat. Compatibility: additive; no route, schema or wire change. Closes #509.

- **Measure the two source-only Exo bounds `#148` had left, and report the one that is not what it
  looks like.** `process_oracle` writes 131,073 bytes and *then* sends EOF, which exercises the
  bridge's read/parse bound; it says nothing about how many bytes a writer gets into a peer that has
  already stopped reading, and nothing about the executor's turn budgets. `bound_oracle` drives both
  shipped entrypoints against the real pinned Exo runtime and a **test-controlled loopback endpoint**
  (so `timeout_millis` can be measured against a reply that is genuinely outstanding) and pins three
  separate boundaries rather than one: a writer offering 131,072 and 131,073 bytes is stopped by the
  bridge's own `exo_bridge_invalid_request` **pre-inference**, with zero model connections; the
  executor reads at most 160 KiB and kills the pipe (`exo_executor_input_bound`), so offering 8×
  that figure is stopped rather than silently swallowed; and — the uncomfortable half — **a handoff
  padded to exactly the executor's 160 KiB read bound is admitted by it and still returns no
  decision**, because the projected model request then trips the extension's *equal* model-write
  bound and every SDK attempt is denied locally before any inference; the case records the
  `fetch_attempts`/`denied_requests` counts it asserts instead of stating a count in prose. The case
  therefore reports that bound as reachable only by a direct drive instead of implying a
  bound-sized handoff yields a decision, and it also measures why the bound cannot be approached
  through the bridge at all: the bridge parses under its own 131,072-byte bound, below the executor's
  163,840, so the read bound is **unreachable through the bridge**, which builds no
  `exo_bridge_input_bound` code path. The projected fields do **not** carry that claim on their own —
  `legal_action_ids` alone is schema-capped at 256 ids of 512 bytes
  (`protocol-artifact/exo-bridge-v1/schema.json`), enough to exceed the bridge's own bound — so the
  case saturates `hard_constraints` only, leaves `legal_action_ids` at fixture size, and records
  that measurement beside both caps rather than a projection ceiling. Both bounds the oracle pins
  are hand-copies of shipped values,
  so the oracle reads the shipped declarations (`bridge/src/main.rs` and `sts2-exo-bridge.rs`) before
  driving anything and fails if either moves, and the report records the shipped values beside the
  pins — no gate other than this one reads the sources those constants mirror. The budget half
  proves `timeout_millis` is a real deadline
  (a 10 s budget aborts a held turn at 10 s and reports the typed `exo_executor_turn_timeout`) and
  that a reply truncated at `max_output_tokens` yields `decision: null` with `exo_turn_failed`
  rather than a fabricated decision. Evidence class: **real pinned Exo process composition with a
  synthetic model and no game** — no provider, credential, game, save or native effect is claimed,
  and the new report carries `full_runtime_admission: false`. Compatibility: tests, documentation and
  one CI step; no shipped boundary changed. Refs #148.

- **Cover the one `RUST002` `#[path]` anchoring branch the suite could lose silently.** The
  `Base`/`bases()` mechanism added in #499 exists to distinguish an *unnested* `#[path]` value —
  relative to the directory of the file carrying it — from an *anchored* one, where an enclosing
  `#[path]` already named the directory. The anchored case lived in one flag with no test: setting
  `modules.rs`'s `anchored: true` to `false` left the whole suite green (**32 passed**) and, on the
  only fixture that reaches the branch, inverted the verdict — it reported `src/thread/other.rs`,
  the file `rustc 1.97.1` actually compiles, and stayed silent on the real orphan `src/other.rs`,
  pointing a `"delete it"` remedy at live code. The repository's own tree cannot exercise the branch
  either: it contains no brace-form `#[path]` at all, so nothing but a unit test can hold it. The
  same mutation now reds
  `inline_path_attribute_inside_a_path_module_is_anchored` (inverted polarity shown above) while
  the other 33 tests stay green, and a second case pins the semicolon form nested in a plain inline
  module. `modules_tests.rs` was at 398 of its 400-line preferred budget, so the `#[path]` family
  moved verbatim into a sibling `path_attribute_tests.rs` — the split already used for
  `traversal_tests.rs` in this package — leaving both files inside budget without an exemption.
  Compatibility: tests and comments only; the rule's shipped behavior is unchanged. Closes #501.

- Stop `RUST002` reporting the children of an inline `#[path]` module as unreachable. A
  `#[path = "thread"] mod m { pub mod child; }` names the **directory** its children live in —
  `src/thread/child.rs` — and rustc reads no file there at all, so the rule's file-only path branch
  fell through and the early return then suppressed the name-based lookup: nothing was reached from
  that declaration, and the module's own `mod.rs` and every child under it were reported as orphans
  with the remedy "delete it". The children now resolve one level below the named directory, at the
  directory of the file carrying the declaration (or the enclosing inline module's directory when
  nested), which is what rustc does. The sibling form is deliberately **not** changed: a `#[path]` on
  a semicolon `mod` always names a file, so a directory value there is a rustc error
  (``couldn't read `src/thread`: Is a directory``), never a miss. Five regression tests, four of them
  failing against the pre-fix rule; `--strict` on this repository is unchanged.

- **Stop `RUST002` from reporting two rustc-valid module shapes as unreachable.** The rule must never
  red a file rustc compiles, and it did so twice. A declaration written with a raw identifier lost
  its edge entirely — `identifier()` read only `r` from `r#move` and stopped at `#`, so the rule
  looked for `r#move.rs` and reported the real `move.rs`, which `rustc` loads (with only `r#move.rs`
  present it fails `E0583` and names `src/move.rs` as the file to create). An inline
  `mod r#type { }` likewise owns `type/`, not `r#type/`. Separately, an `include!`d file carried the
  **includer's** directory forward, so a `mod child;` written inside a fragment was resolved beside
  the includer instead of beside the fragment that is compiled in place; that inverted the finding
  in both directions, missing the includer-local decoy and flagging the fragment-local file. The raw
  form and the fragment's own directory are now both honoured, each with a regression test proven to
  fail against the pre-fix code. The first defect was live: `sts2-game-mod` declares `mod r#move;`
  beside a compiled `move.rs` and was red for it. Compatibility: analysis only; no runtime,
  provider, game, or native behavior changes. Closes #495.

- **Deny the two rustdoc lint classes the doc gate was only warning about.** The `Check documentation
  links` step denied only `rustdoc::broken_intra_doc_links`, so it exited 0 while printing `generated
  5 warnings`: four `private_intra_doc_links` sites where public documentation linked to a private
  item and resolved *only* because `--document-private-items` was passed (`final_budget_prepare.rs`,
  `capability.rs`, `contract_commands.rs`, `research_inspection/mod.rs`), and one
  `redundant_explicit_links` target (`membership_render.rs`). The step now also denies
  `private_intra_doc_links` and `redundant_explicit_links`, and the five sites are repaired by
  unlinking the private or redundant target while keeping the prose — except
  `capability.rs`'s `[`Self::profile_name`]`, which is `pub` and therefore left as a working link.
  A private link can only resolve under `--document-private-items`, so it breaks for every external
  consumer even while the gate is green; escalation is the point, not the warning count.
  Compatibility: CI and doc comments only; no production code, schema, route or behavior change.
  Closes #489.
