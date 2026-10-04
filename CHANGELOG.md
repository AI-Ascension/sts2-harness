# Changelog

All notable user-visible or operational changes to this project are documented here.

The project follows Semantic Versioning once versioned releases begin. Foundation work does not
claim a released harness version or runtime compatibility.

Completed entries that no longer fit the active file's preferred size budget are preserved verbatim
in [`docs/CHANGELOG-ARCHIVE.md`](docs/CHANGELOG-ARCHIVE.md) and the dated archives beside it,
including [`docs/CHANGELOG-ARCHIVE-2026-09-28.md`](docs/CHANGELOG-ARCHIVE-2026-09-28.md).

## Unreleased
- **A management connection that ends without delivering its response is now reported through an
  injectable sink, and the report carries attribution.** The line was an `eprintln!`, and an
  `eprintln!` cannot be asserted: deleting it passes every other test in the crate, which is how the
  gap survived 524 tests. It also named no connection — no peer, no code — so a report found in a
  red CI log could not be tied back to a request, which is the property the line exists to provide.
  `ManagementServer::start_with_diagnostics` now accepts a `DiagnosticsSink`;
  `ManagementServer::start` delegates to it with `StderrDiagnostics`, so the production path is
  unchanged and the emitted line gains the peer's address and the management error's stable code.
  A connection that *was* answered reports nothing, so the sink cannot bury a real fault in noise.
  Refs #819.

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

- **The advisory plan's dispatch contract is now tested, and one of its guarantees was being
  enforced by its callers rather than by the plan.** `ActionPlan` carried the whole #319 contract
  with no tests beside it: a plan is a prediction about a state that has not happened yet, so a step
  the host no longer offers, a state the plan did not assume, a generation the host has not left, or
  an unsettled effect must each *end* the plan rather than coerce, substitute or retry it. Every one
  of those properties was previously asserted only by reading the source. Nine tests now drive the
  real `ActionPlan`, each constructed so the guard it names is the only thing that can end the plan
  — the first version of the suite passed under every mutation, because a second guard fired first
  and returned the same `None`.

  That isolation found a real defect. `ActionPlan::action_completed(false)` cleared the remaining
  steps, but every caller that received an unsettled result also dropped the entire plan before
  asking again, so the clear was unreachable: the next dispatch returned `None` at the `settled`
  check whether or not the steps were still there. The guarantee was real only because callers
  remembered it. The plan now owns the disposal and the test asserts on the plan's own contents, so
  a caller that forgets cannot dispatch on a prediction whose outcome is unknown. No caller's
  behaviour changes.

  A second test covers the case that records most easily get wrong. `DecisionRecorder` already marks
  each dispatch with `reused_model_execution`, but a **dropped** plan and a **kept** plan dispatch
  the same action, so the row reads identically whether the prediction was right or the plan was
  abandoned and the provider was asked again — different claims about a run. Driving the real
  `ExoDecisionSource` over a scripted transport, the undercut case must now cost a second round
  trip and record `reused_model_execution: false` under the new execution ID; it fails exactly when
  later plan steps stop revalidating, which no other test in that module does.

  No new plan shape, wire format or schema: #319's schema, dispatch-time validation and recording
  tasks were already implemented in `exo/decision.rs`, `episode/action_plan.rs`,
  `episode/policy_router.rs` and `runtime_v3_recording.rs`, and adding a second plan type would have
  left two answers to "what may be dispatched next" with the weaker one on the dispatch path.
  Evidence is local unit and integration testing over a scripted transport; no native host, live
  provider, hosted or production result is claimed. Refs #319, see ADR 0079.


- **A disclosed card the request builder would refuse is now dropped instead of killing the whole
  exchange.** `cards()` filtered a card set on a strict subset of the rules `validate_options`
  actually enforces, so a set that survived the filter could still be refused with `InvalidOption`
  — and that refusal takes down the entire action request, including the well-formed action
  question and its answer, over a card the operator never saw asked about. A non-printable
  `choice_id`, a repeated `choice_id`, an over-long description, and a description carrying a
  control character all reached the refusal this way. Admissibility is now decided by the same
  predicate the builder refuses on, so the filter and the refusal cannot drift apart again, and a
  repeated identity is offered once rather than refused. The action decision is still delivered in
  every case, and a set whose every card is unusable still yields no card question at all.
- **A System One request is now bounded as a whole, not only as its state plus its longest
  question.** ADR 0053 publishes two provider limits — `64k` tokens per request, of which `32k`
  covers the state plus the longest question — and the builder enforced only the second.
  `longest_question_bytes` returns the maximum over questions, never the sum, which was correct
  while every request carried one question and silently wrong once #805 made the map a set: a
  caller could fill the set towards `MAX_QUESTIONS` with every individual question inside the
  shared budget and still send a request past the per-request ceiling, where the provider would
  answer `422` on a request this builder exists to refuse locally, on a long run, with nothing in
  the diagnostic to tell that refusal from any other provider error.
  `build_choice_questions_request` now also enforces `MAX_REQUEST_BYTES` (`128 KiB`) and reports it
  as the new `RequestTooLarge` rather than as `OverBudget`, whose text names the state-and-question
  bound and would misreport which one was hit. The constant is deliberately **twice**
  `MAX_STATE_AND_QUESTION_BYTES`: ADR 0053 states the per-request limit in tokens rather than
  bytes, so it is translated under the same two-bytes-per-token reading the existing constant
  already documents, and reusing the smaller number would have encoded `32k` where the provider
  publishes `64k`. That reading is the conservative one — at three or four bytes per token the real
  ceiling is nearer `192 KiB` or `256 KiB` — and it keeps the refusal on this side of the provider,
  which is the trade the sibling bound already makes. The bound is taken over the compact
  `serde_json` bytes of the value the builder returns, the same form the bridge writes and sends, so
  it measures what is actually transmitted and cannot be bypassed by a question appended afterwards.
  **This is a latent contract gap, not a live outage, and is stated as one:** the only caller asks at
  most two questions against `2.5%`–`6.5%` of the shared budget, so the new bound refuses nothing
  today. No consumer pin moves — `system_one_questions_digest` covers the `questions` object rather
  than the body, so every previously admitted request is byte-identical and every recorded digest
  still matches. ADR 0078 records the new bound and its reasoning. Refs #806, #805.

- **The provider-session capability descriptor's `evidence` field is renamed `provenance`, because
  it never gated anything and the old name said it did.** `NativeCapabilities::validate()` has
  never read the field, so every value was equally admissible, yet `evidence` sat beside genuinely
  request-selectable axes and read as a four-step ladder ending at `LiveProvider`. A caller writing
  `capabilities.evidence == CapabilityEvidence::LiveProvider` got a true answer for a descriptor
  whose adapter never contacted a live provider, and the compiler could not warn, because the
  variants looked like a closed, ordered set. The variants are not an ordering: they mix what was
  compiled, what was executed, and what was contacted, and `live_provider` asserts a runtime event
  outside the process that no field in the descriptor can witness — so no policy floor can be
  honestly built from it, and `policy.schema.json` now records that it deliberately has no such
  field. Both contracts state the non-admission role in their own text rather than leaving it to be
  inferred. **This is a wire-format break:** the schema is bumped to
  `ascension.provider-session.capabilities.v4` (revision `harness-provider-session-v4`), so a peer
  still emitting `evidence` is rejected by the `deny_unknown_fields` decoder rather than silently
  accepted. Integrity is not weakened: `descriptor_digest()` still covers the field, so a descriptor
  cannot be relabelled without invalidating its own digest. `NativeBinaryFakeUpstream` is retained
  and documented as unwired rather than removed, since removing it would break a peer outside this
  tree. Refs #755, #109.

- **The advertised `context_modes` axis is documented as build provenance, not a request axis.**
  It is the one axis in `capability_fields()` with no request-side counterpart: no wire request
  field can select a context mode, so `Continuity` is unreachable and the preflight branch that
  guards it cannot run in production. The field's name and its place beside genuinely
  request-selectable axes invited the opposite reading, and the owner decision (#760) is to keep
  the axis and say plainly what it is — the descriptor and the guard are two reads of the same
  `SUPPORTED_CONTEXT_MODES` constant, so this is a build property published so a caller can see
  what one build implements. The same words now appear in the constant, the descriptor field, the
  preflight guard and #757's reachability module, and a test reads all four so a one-site edit
  fails instead of quietly restoring the ambiguity. The guard is deliberately retained: it is the
  only place a future build that genuinely adds a non-fresh mode would be caught. Making the axis
  request-selectable is not decided here and remains #109's to carry. Refs #760, #757.
- **The System One bridge performs its own HTTPS exchange, so a run carries one digest instead of
  two.** It previously spawned an operator-owned transport named by `--transport`, so a run pinned
  two artifacts and the runtime verified one. It now uses a pinned `rustls` client with trust
  anchors compiled in from `webpki-roots`, so a run record can name which roots verified the peer.
  `--transport` is gone and the runtime admits `["--model", MODEL]`, so a stale four-element
  configuration is refused rather than silently narrowed. The handshake completes before a
  request is built, so a certificate or hostname refusal discloses nothing; the deadline bounds
  connect, handshake, write, and read in both directions; the `128 KiB` bound applies to the body
  against a checked `Content-Length` or `chunked` framing, so a truncated response cannot be
  parsed as a whole one; a TCP close without `close_notify` is a refusal rather than an end of
  message; and a missing, empty, oversized, or header-unsafe credential is refused before a
  socket is opened instead of becoming an empty `Bearer` header. Non-`200` refusals (including
  `429` and `529`) fail closed, and a TLS failure is never a downgrade or a retry. Neither
  `rustls` backend is pure Rust — `ring` 0.17.14 carries 17 `.c` and 73 `.S`, `aws-lc-sys` 0.39.0
  662 `.c`/`.h` and 849 `.S` — so that part of the issue was unsatisfiable as written; `ring` was
  taken as the smaller, and this workspace already compiles C for `rusqlite`. The compiled-bridge
  evaluation lane narrowed with it: it staged a synthetic `--transport` the bridge no longer
  reads, so its nine provider-answer cases are retired rather than left failing. Response classes
  are asserted in-process against bytes and the ordering a real peer shows against a loopback TLS
  server, which leaves no automated lane driving a provider exchange through the compiled binary
  end to end; ADR 0053 records that gap and the seam that would close it.

- **A baseline-fence refusal now reports presence, not disagreement.** Since #468 the save-profile
  fence rule is presence-only: the owner checks the profile identity and the baseline independently
  and imposes no equality between them, so a distinct-but-valid fence can no longer be refused.
  `ProfileSetupError::BaselineFenceMismatch` means only that a required fence was absent or a
  prohibited one supplied, and its doc comment already said so — but the router-visible `Display`
  string still read `save-profile baseline fence does not match the operation`, the superseded
  meaning. A router branching on a refusal, or an operator reading one, was told the fence disagreed
  with something when a required fence was missing or a prohibited one supplied. The string is now
  `save-profile baseline fence is required or prohibited by the operation`, and a table test pins
  the whole `ProfileSetupError` vocabulary, asserting no refusal leaks a supplied profile value, a
  host path or game text. The admission rule and #102's T2 adapter slice are unchanged and remain
  open. Refs #102.

- **A transport that succeeds with a malformed body is reported as the parse error.** #752 read
  the exit status ahead of the worker joins and fixed the *refusal* case, but on a successful exit
  the writer's `EPIPE` still pre-empted the body-parse error: a transport that answers without
  draining its stdin — one answering from a cache, or short-circuiting — closes the pipe early,
  so the named cause was `Broken pipe (os error 32)` or the parse error depending on which finished
  first. Measured on `main` at `82259540`, 40 runs per case: 27 wrong / 13 right, unchanged across
  the #752 fix, so this was its residual rather than a regression. Once the child has exited
  successfully the status has classified the run and the response is waiting in the stdout pipe,
  so the writer's `EPIPE` is no longer propagated; a non-`EPIPE` I/O failure or a panicked worker
  still is. The new case runs the exchange 12 times and asserts `Broken pipe` never appears and
  that every run names the same cause — an invariant, since pinning either outcome alone would
  flake in the direction it forbids. Verified non-vacuous: with only the production change
  reverted it fails on run 0. Refs #753, #751.


- **A transport that refuses a request is reported as the refusal, not as a broken pipe.**
  #746 made the bridge print `cause: {error}`, but the workers were joined before the transport's
  exit status was read, and a transport that exits without draining its stdin fails the writer's
  `write_all` with `EPIPE` — so `??` returned on that plumbing artifact and the `status.success()`
  check never ran, so an operator transport declining a request with a non-`200` was reported as
  `cause: Broken pipe (os error 32)`. The exit status does not depend on scheduling, so it is read
  first; the workers are still joined, so a real transport I/O failure is still surfaced. New case
  `a_refused_transport_is_reported_as_the_refusal_and_not_as_a_broken_pipe` asserts the cause is the refusal and is not `Broken pipe`. Refs #751.
- **A transport that could not be given a worker thread was never orphaned, and is now moot.**
  The writer arm returned a failed thread spawn with a plain `?`, by which point the transport was
  already running, and `std::process::Child` has no `Drop` that signals the process — so it reported
  the failure correctly and orphaned the child anyway. #750 routed both arms through one
  `kill_child` helper, asserted at the real call sites — a first attempt was vacuous, since deleting
  either arm's left the suite green. **Superseded, not regressed:** #758 removed the child entirely
  — the bridge now exchanges in-process over a `TcpStream`, with no operator-owned `--transport`
  subprocess — so there is nothing left to orphan.
  `kill_child` and the three tests that drove refusals through `exchange` went with it in
  `08a47648`; coverage moved to `jev_tls_transport_loopback_tests.rs`, which completes a real
  handshake against a loopback TLS peer and refuses an unknown CA, a peer that closes mid-handshake,
  and one that never answers. Refs #748, #758, #299.
- **A bridge that never launched now says so, instead of posing as a behavioural refusal.**
  `main` was `if run(&options).is_err()`, which discarded the error entirely, so every refusal —
  whether the provider answered wrongly or the host could not fork — printed the same single line
  and exited 2. `main` now prints the cause on a second line, leaving the first line's contract
  untouched: the suite asserts `stderr.contains(FAILURE_LINE)`, not equality.
  The second half is the larger of the two. `exchange` started its two pipe-servicing threads with
  `std::thread::spawn`, which *unwinds* on `EAGAIN` rather than returning it — under `ulimit -u 1`
  it dies `panicked at std/src/thread/functions.rs:131` with exit 101, where `Builder::spawn`
  returns `Resource temporarily unavailable (os error 11)`. Both workers now use `Builder::spawn`,
  so a host that cannot create them is reported as the transport failure it is, and the child is
  killed if the *second* worker cannot start, never orphaning the first on a dead pipe.
  That panic is a real sibling failure, but **not** what this CI event was, and the reason is the
  test's own control flow rather than the log. The failing case (run `36338651404` attempt 1, at
  `fd4dc88d`) reported the non-vacuity guard's own message — "raw-503.sh refused before the
  provider answer arrived, so the case proves nothing" — which is only reachable *after*
  `refused_without_a_decision` has already confirmed the run exited nonzero, wrote nothing to
  stdout, printed the bridge's failure line, and exited exactly 2. So the bridge refused cleanly,
  and the suite's `101` is the harness reporting a failed assertion, not an abort. A panic is
  independently excluded: that attempt's log contains no `panicked at`, and the failure arrived as
  a returned `Err` (`7 passed; 1 failed`) rather than an unwind. The run names no errno, so *which*
  launch failed is not established; serialising the launches removes the pressure either way, since
  each invocation costs a child process plus two threads.
  This does not claim the flake is fixed. It removes the two paths on which the bridge could not
  tell a launch failure from a refusal, and the guard that fired in CI stays armed — the new
  end-to-end case asserts a transport that cannot be launched is refused *and* names a cause other
  than the one every behavioural refusal carries, so a case driven past the transport cannot pass on
  a refusal produced by a cause it never reached. The guard is unchanged and still fires when a case
  genuinely fails to reach the provider (confirmed by mutation). Refs #645.
- **The census now reads every page of a listing, not just the first.** The listing of merged pull
  requests was requested with `page=1` hardcoded and no pagination loop, so any repository whose
  merged pull requests ran past one page was reported as having only the merged pull requests on
  page one — and the run still exited zero. Measured on this branch before the fix,
  `sts2-harness` reported `merged=62`; the repository actually has **452** merged pull requests
  across 6 pages of 100, so 390 (86.3%) were silently lost and `unreadable=0` claimed the
  measurement was clean. This is the same class of silent under-report the tool was written to
  prevent, arriving through a different door: the transport named the objects it could not read,
  but nothing noticed the objects it never asked for. The traversal now follows the server's own
  `rel="next"` Link header until the server stops offering a successor, so it ends where GitHub
  says the listing ends rather than at an assumed page count, and a page that cannot be read
  fails the run closed with that page's own identity. Each repository and the run total now also
  report `pages=`, the size of the traversal actually performed, so a one-page measurement cannot
  be mistaken for a complete one. Refs .github#49.
- **A census that names what it could not read.** `tools/repo-census` measures merged pull
  requests and their review record org-wide, and treats "the tool exited 0 but its own output
  does not parse" as a named, non-retryable outcome rather than an empty result. `gh api` 2.23.0
  does exactly that on a compact response containing a backslash escape — it re-serialises with one
  extra backslash, so the document is invalid JSON — and a census built on it silently loses the
  object, and loses the whole page when the object sits in a paged listing. Measured on this branch
  across all 18 repositories: 636 merged pull requests read, 104 carrying a review pinned to the
  merged head, 6 carrying reviews pinned to a superseded commit, 526 with no review at all, and
  exactly 1 page unreadable (`sts2-game-mod` page 1, which the previous tooling would have reported
  as that repository having 0 merged pull requests). **Those totals were themselves measured with
  the single-page defect described above and are therefore undercounts for every repository with
  more than one page**; they are left here as originally recorded rather than restated, and
  re-measurement belongs with the fix. The CLI exits non-zero when anything could not be read, so
  a gate built on it cannot report a clean measurement over a partial one. Refs .github#49,
  .github#50.
- **The review-of-record gate is no longer unmergeable after its own review lands.** The
  `concurrency` group added for #729 was keyed on the pull request number and head SHA but not on
  the event name, so a `pull_request` run and a `pull_request_review` run for the same head landed
  in the same group. Measured on #733 head `4b20402e`: push run `36447039873` was created at
  15:53:46Z and ran until 15:57:28Z, and review run `36447155603` — the **newer** of the two — was
  created at 15:54:40Z and cancelled at 15:54:56Z. `cancel-in-progress` gated on the event name did
  not prevent it, because that expression only decides whether the *arriving* run supersedes; it
  cannot stop an in-flight predecessor from cancelling the arrival. Since the review trigger is
  the only path that turns the check green, the head was left carrying a cancelled
  `review-of-record` beside a green run of the same context, and under ruleset 24104281 the
  required check resolved to the cancelled run, so the merge API refused. The group key now carries
  the event name, giving the two triggers separate groups: a review submission can no longer be
  cancelled by a push, while a push still supersedes its own earlier push run and the head SHA
  still prevents a push from cancelling a previous head's run. Refs #735.
- **A durable exchange that completed is no longer reported as a turn timeout.** The admitted
  transport re-measured the caller's ceiling against its own wall clock *after* the inner
  transport returned, so an exchange the inner effect had already completed inside its own
  deadline was reported as `Timeout` whenever the surrounding host was slow. That is the exact
  ambiguity the admission layer exists to remove: the inner transport already owns the deadline
  and can tell an overrunning peer from a slow scheduler, so the adapter now passes the ceiling
  down and surfaces the inner verdict instead of re-deciding it. The admission tests that assert
  a *successful* exchange were also handed the fixture's own effect budget, which made "the host
  was slow" indistinguishable from "admission refused"; they now use a ceiling above the effect's
  budget, and timeout behaviour is asserted deterministically against a peer that overruns.
  Refs #716.
- **The review-of-record gate's runs are now serialised per head.**
  `review-gate.yml` triggers on `pull_request` as well as `pull_request_review`, and without a
  `concurrency` group a slow push-triggered run overlaps the review-triggered run for the same head
  instead of yielding to it. The group is keyed on the head SHA as well as the pull request number,
  so a push never cancels work belonging to the previous head, and `cancel-in-progress` is gated on
  the event name so a review submission is never cancelled, since that trigger is what turns the
  check green. This is queue hygiene: it does **not** clear a completed red run, and it does not
  make a required check resolve against the newest run. Refs #729.
- **The process-group timeout test no longer fails against correct code.** `kill -0` succeeds for
  a **zombie** as well as a running process, so a reaped-but-uncollected descendant read as alive.
  It failed 5 runs in 20 on fixed `main`; the probe now also reads the process state. Refs #722.
- **The `grandchild_gh` stub no longer leaks a temp directory per run.** `#718` fixed the
  process-group kill and its new test helper created a `$TMPDIR` directory per call that nothing
  removed, reintroducing the `#713` leak one PR after that issue was closed. It accumulates
  separately from `#713`'s because the name prefix differs (`-grandchild-` vs `-test-`), and it
  fires on every run of the suite. The helper now uses the `TempDir` owner that `#713` added for
  exactly this shape, and a second control asserts the new call site, because the existing one
  covered only `fake_gh` -- which is why the leak passed a property that was already green.
  Refs #719.
- **Give the `gh api` call the 60-second timeout the reference has.** The reference bounds every call; the Rust port used `Command::output`, which waits forever, so a hung `gh` spends the whole `timeout-minutes: 5` job.
  It fails closed rather than wrongly, so the cost was availability, not correctness. A timeout
  kills the whole process group, not just the child, so helpers `gh` spawned do not survive
  holding the pipes. Refs #702.
- **Own the `src/bin` loopback port until the child that binds it is spawned.**
  `runtime_v3_game_information_entry_support.rs` drew its loopback address with a
  `free_loopback_address()` that read `local_addr()` and dropped the listener inside the same
  expression, so the port was unowned from that instant until the child bound it. That is the
  `#673` defect; `#693` fixed it in `tests/support/**` only, and deliberately did not reach
  `crates/harness/src/`.
  It is reachable end to end. `run_runtime_entry` puts the address in the owner config as
  `management_listen`, the runtime child binds it, and `wait_for_management` then polls
  `ManagementClient` until the route answers -- so a port taken in the window left the child's
  `bind` to fail with `EADDRINUSE` while the readiness check reported success, because it
  connects to whatever is listening rather than to the child.
  The two remaining call sites needed different fixes, and using one shape for both would have
  been wrong. `peer_encoding_tests.rs` rebinds in-process on the very next statement, so it now
  binds `:0` once and keeps that listener; there is no window to close and a reservation there
  would be pure ceremony. `entry_tests.rs` hands the address to a child, so it holds the listener
  in a `ReservedAddress` and releases it as the last statement before `Command::spawn()`.
  `release` is idempotent, which is what lets the replay pass rebind one address across two
  spawns without a separate flag. The allocator's prescribed fix -- returning the bound
  `TcpListener` -- is the same framing the `LivePeers` entry below records as measured and
  refuted on this kernel, so both sites take the reservation shape `#693` landed.
  The regression tests include `the_pre_fix_shape_really_does_leave_the_port_unowned` as a
  negative control, so the reservation test cannot pass for the wrong reason. No test here is
  known to fail this way and none is claimed to; the fix is on the verified code shape and the
  verified reachable call path. Refs #681, #673.
- **Stop the review gate's own test suite from failing on `ETXTBSY` while a stub is being
  written.**
  `review-of-record` is a required check on every pull request and it runs the gate's own tests, so
  a test-only race here blocked unrelated work at random — including after the author had already
  posted a correct review, which is the one signal that makes people re-run instead of read. Run
  `36386964129` failed on `tests::runner::head_sha_extracted_from_payload` with `Text file busy
  (os error 26)`, and run `36389325307` attempt 1 failed the same way on the same head that attempt
  2 and run `36389016026` passed: identical commit, opposite verdicts.
  The mechanism is **not** a path collision, and the per-call nonce `#700` added is not what was
  missing. `fs::write` closes its handle before returning, so nothing leaks: the window is the few
  microseconds between that close and `execve`, and `Command::output()` reaches the stub through
  `fork` + `exec`. If any *other* test thread is inside its own write-to-exec window at that
  instant, the forked child inherits that thread's still-open write descriptor, and the kernel
  refuses to `execve` an image any live descriptor holds open for writing. Each stub has its own
  directory and filename, so no per-call uniqueness can prevent it — the coupling is the inherited
  descriptor, not the name. Reproduced deterministically with distinct paths and distinct inodes:
  thread A holds its own stub open for writing, thread B forks 60 children on its own stub, and
  A's `execve` fails `ETXTBSY` 3 times in 3. That also makes the earlier diagnosis on the issue
  wrong in a way that matters: its supporting control — "8 threads rewriting and execing *their
  own distinct path*, 0 failures" — never put a `fork` inside another thread's write-to-exec
  window, so it reproduced the clean case and was read as the dirty one.
  This repository had already measured and fixed exactly this for its own written stubs:
  `tests/support/runtime_v4_executable_composition_process/spawn.rs` records **5 failures in 240
  spawns** without a retry and **0 in 240** with one. `tools/review-gate` never adopted it, which
  is why the one caller still exposed was the required check's own suite. The bounded retry is
  ported for the same measured reason, and `Command::output()`'s stdio wiring is reproduced
  explicitly, since a bare `spawn` would otherwise capture no output and fail every read closed.
  Retrying is sound because the offending descriptor is always closed by its owner, so the
  condition is transient and the deadline cannot be outlived by a persistent one.
  The regression manufactures the condition rather than looping the suite and hoping, which is what
  let this survive `#700`: that fix was real, but was verified with **1 failing run in 8 against
  the old code and 0 in 12 after**, a rate that cannot distinguish "fixed" from "almost never hit"
  — one failure in 40 full-suite runs here. The new test holds a write descriptor open on the stub
  it is about to exec, releases it the way a forked child would, and asserts the runner still
  succeeds; verified to fail against the pre-fix implementation. A companion control asserts an
  un-retried `Command` on the same held-open stub still fails with `ETXTBSY`, so the regression
  cannot pass by never provoking the condition.
  Refs #707, #668, #609, #700.
- **Run every test binary in the CI test step instead of stopping at the first failure.**
  `cargo test` executes each test target as a separate binary and aborts the whole invocation at
  the first failing one unless `--no-fail-fast` is passed, and this crate's integration tests are
  individual binaries that cargo runs alphabetically by filename. On the first attempt of
  `Continuous integration` run `36338651404` at `fd4dc88d`, a single failure in
  `jev_bridge_process.rs` (position 128) stopped the run after 128 of the 264 top-level test
  binaries, so 136 never ran — including `served_gateway_capture_drain.rs` (position 245),
  `runtime_v4_executable_composition.rs` (position 222) and `workflow_store.rs` (position 264, the
  last) — and the rerun then went green, certifying only that every binary *up to the first
  failure* passed. The step now passes `--no-fail-fast`, which adds no retry and hides nothing: a
  genuinely failing test still fails the step and the job on its own merits. A new check asserts
  the invariant rather than the flag text — any workspace-wide `cargo test` must carry the flag —
  and carries a vacuity guard so a sweep that matches nothing cannot report success. Refs #645.
- **Install the review gate in `sts2-harness`.** `CONTRIBUTING.md` says a green run does not
  substitute for review, but nothing here enforced it: every workflow this repository ran was a
  product or policy check, none of them a review gate, and merges landed here with no review of
  record. The gate exists and is reviewed in `AI-Ascension/.github` (`review-gate.yml` plus
  `tools/review_gate.py`), and until this change it was installed in exactly one repository in the
  campaign -- the one that ships no product code. `sts2-harness` is where the gate belongs: it
  carries the large majority of the campaign's merges, and the large majority of the ones that
  landed with no review of record.
  It is installed as a new Rust workspace tool, `tools/review-gate`, plus `review-gate.yml`, and
  not vendored verbatim: this repository's `LANG001` rule prohibits Python source and
  `repo-policy --strict` enforces it, so the reference could not be copied without breaking the
  build. The decision logic is a case-for-case port, and the ported suite pins every branch the
  reference's own 25 tests pin -- the head-pin rule, the `COMMENT`-state rule, the
  all-reviews-not-just-latest regression, and every fail-closed path.
  The workflow checks out the **default branch** and runs the gate from there, so a pull request
  cannot edit the gate into passing itself: the one property worth more than the convenience of
  testing the pull request's own copy. The two halves therefore had to land in order -- the tool
  first, then the workflow -- because a single combined pull request would have checked out a
  `main` holding neither the tool nor its tests, and would have failed for a reason that has
  nothing to do with review. The job also runs the gate's own tests from that same default-branch
  checkout, so the gate cannot be weakened on `main` without turning its own check red. Refs #668,
  #49.
  One test-harness defect found by CI while landing it, and fixed here rather than papered over:
  the fake `gh` the `runner` tests execute derived its temp path from a hash of the *body* it would
  print, so two tests that legitimately pass the same body -- `real_subprocess_roundtrip` and
  `head_sha_extracted_from_payload`, both `{"head": {"sha": HEAD}}` -- resolved to the same file.
  One `execve`d the stub while the other was still writing it and lost with `ETXTBSY` ("Text file
  busy"). The name is now unique per call from an atomic counter, so the outcome no longer depends
  on scheduling: reproduced 1 run in 8 against the old naming, 0 in 12 after.
  Refs #668, #49.
- **Assert that a required structural marker occurs exactly as often as policy declares it.**
  `check_required_preamble` walked the file with an ordered scan that stopped consuming markers once
  they were all satisfied, so a *second* copy of a marker past the last one was never examined. A
  changelog that repeated `## Unreleased` with the six preamble lines copied above it satisfied
  every assertion the rule made, which is how a duplicate shipped in #676 and survived #680: the
  first marker check tested presence and order, and neither of those is uniqueness. The scan now
  counts every occurrence of every marker and reports a `DOC003` finding when a file supplies more
  copies than `policy.toml` declares. The comparison is surplus over *declared*, not a hardcoded
  one, so a policy that deliberately declares a marker twice is still satisfied by two headings and
  still reports a third as surplus. The duplicated preamble and heading themselves are removed here,
  and the changelog's waiver count is restated to match the cleaned file. Closes #682.
- **Split the control authority under the production size limit instead of acknowledging the
  breach.** `context_control/state.rs` sat 171 nonblank lines over `rust_production_max` and was
  held there by a `policy.toml` exemption. The one `impl ControlAuthority` had grown four separable
  concerns: the gate transitions, operation admission and settlement, the durable journal
  boundary, the idempotency/event-ledger bookkeeping those transitions share, and the digests they
  depend on. They are now `state/gate.rs`, `state/operations.rs`, `state/journal.rs`,
  `state/ledger.rs`, and `state/digest.rs`, leaving `state.rs` as the authority's types and its
  remaining stop, boundary, and plan-admission transitions. The
  exemption is deleted rather than reworded, so the size rule measures these modules for real
  instead of waiving them — which removes the last exempted breach the repository carried. This
  completes the three splits that `#640` tracks. No public item was added, removed, or renamed; the
  split is verified method-for-method against the pre-split file. Closes #574.
- **Split the provider renderer under the production size limit instead of acknowledging the
  breach.** `context_control/render.rs` sat 199 nonblank lines over `rust_production_max` and was
  held there by a `policy.toml` exemption. The renderer carried four separable concerns that had
  grown into one file: budget accounting, admission of context items, Exo request conversion, and
  the shared Ollama projection. They are now `render/limits.rs`, `render/admission.rs`,
  `render/exo_request.rs`, and `render/ollama.rs`, leaving `render.rs` as the boundary that
  declares and re-exports them. No public item was added, removed, or renamed — the split is
  verified item-for-item against the pre-split file — and the exemption is deleted rather than
  reworded, so the size rule now measures these modules for real instead of waiving them. This is
  the same treatment `#643` gave the provider-session metadata store, and it removes the largest
  of the three breaches that `#640` tracks. Closes #572.
- **Stop requiring a waived breach to exist before the policy gate will pass.** The
  `repository_reports_its_waived_breaches` test exists so the `EXEMPTED` finding cannot be computed
  and then silently discarded — the `#569` defect. It asserted both that the reported count matches
  the recomputed one *and* that at least one `EXEMPTED SIZE001` line appears. Once the last size
  exemption was replaced by a real split, that second assertion failed on a repository that is now
  fully compliant: there was correctly nothing left to waive, and the gate reported it as an error.
  The counting assertion — the one that actually catches a discarded finding — is unchanged, and a
  new assertion now pins the zero case explicitly, so the test still fails if a waiver is dropped
  from the output and still fails if one is invented where none is owed. Refs #574.

- **Match `DOC003`'s structural markers as whole lines, in order, anchored to the file's opening
  line.** The rule tested each marker with `text.contains`, so a marker was satisfied by its own
  string appearing anywhere in the file — including inside prose describing it. `CHANGELOG.md`
  documents its `## Unreleased` heading in prose, so deleting the real heading while that sentence
  survived reported nothing: root measured `0 warning(s), 0 error(s)` and exit 0 on a `CHANGELOG.md`
  with the heading deleted and the nonblank line count held at 493, which is the exact false green
  the rule was added to end. Markers now match whole lines, are consumed in declared order so one
  repeated heading cannot stand in for the preamble, and the first marker must be the file's first
  nonblank line — which is what the doc comment on `check_required_preamble` already claimed and
  the substring test never enforced. Closes #620.

- **Refuse to ship a conflict marker, because deleting the one that reached `main` proved the gates
  would not.** A bare `=======` sat in this file at line 29, left by the `#601` merge as a
  three-way-resolution artifact with no `<<<<<<<` or `>>>>>>>` counterpart, and it survived a merge
  and a full round of green CI. It survived because both gates covering this file are shaped so that
  it passes: the size rule scored the removal as a *reduction* in nonblank lines, and `DOC003`
  asserts that markers are *present*, not that conflict debris is absent. #618 removed this
  instance incidentally, while re-resolving this file for an unrelated entry; deleting it fixes one
  occurrence and leaves the class open, because the next rebase across this boundary puts it back.
  The class is not hypothetical. Each instance is cited by the commit that *introduced* it, and
  each removal by the commit that removed it. No clause rests on a branch, a rebase, or a count, and
  every citation is a commit rather than a branch tip, because a tip can be closed or superseded
  while the prose still names it. `main` is the only ref whose state is asserted.
  `501a711`, the `#601` merge, introduced the line-29 artifact, reached `main`, and `06eba7e6`
  (`#618`) removed it; `8e3ffea` added a second at line 45, which no commit in its own history has
  removed and which is not on `main`; `696e56f` added one at line 60, which `cba9be8` removed; and
  `df9ef25` added one at line 100, which `94802b8` removed. Of the four, only `501a711` is on `main`.
  `repo-policy` gains
  `CONFLICT001`, which reports any tracked text file with a line *starting* with seven or more `<`,
  `=` or `>`, naming the path and the line number so review can act on it.

  The match is positional rather than a substring search, because `contains` would fire on prose
  that merely names a marker — the defect `#620` is fixing in `DOC003` — and an equals run inside a
  sentence is ordinary text. It is anchored at the start of a line rather than requiring the line to
  be *entirely* a run, and that is the difference between a gate that works and one that does not:
  git writes its opener and closer with a label attached (`<<<<<<< HEAD`, `>>>>>>> topic`), so a
  whole-line test would match only the bare separator — the one case that happened to exist here —
  and would then stay silent through every future rebase leaving a complete block behind.
  Indentation is not a marker, since a merge never indents one and an indented run is an indented
  code sample.

  Fenced code blocks are deliberately **not** excluded. Excluding them would exempt the
  documentation of this very defect while still letting a real artifact inside a fence through, and
  prose about a marker never puts one at column zero, so the common case is clean without the
  exclusion. A setext underline of seven or more is reported, because git's separator is exactly
  seven and any longer threshold would miss the defect; this repository uses ATX headings
  throughout, so nothing in the tree is near that boundary. Verified by reproduction in all three
  marker forms under `--strict`, not by construction alone. Refs #624, #625.
- **Assert the changelog's own structure, because a size gate cannot see the loss of it.** The
  `#606` merge resolved a `CHANGELOG.md` conflict by keeping the bullet list and dropping the title,
  the preamble and the `## Unreleased` heading, and every gate stayed green: all twelve deleted
  lines were nonblank, so the size check scored the deletion as a *reduction* and passed. The file
  stopped identifying itself, and with the preamble went the pointers to
  `docs/CHANGELOG-ARCHIVE.md` and the dated archives — leaving the active changelog
  structurally indistinguishable from an archive, under an exemption that still called it "the
  active changelog". #611 restored the text; this adds the check that was missing. `policy.toml`
  gains a `project.required_preambles` table mapping an exact path to the markers that file must
  carry, and `repo-policy` reports `DOC003` for any that are absent. It is deliberately not a size
  rule and deliberately not an exemption — a waiver is the mechanism that let the original defect
  hide, so the assertion is a hard error on an exempt file too. Verified by reproduction, not just
  by construction: with the preamble deleted, `repo-policy --strict` reports both missing markers;
  restored, it is clean. Three tests cover the present case, the exact regressed shape (a bare
  bullet list), and a file outside the table. Closes #614.
- **Pin the shared 8 MiB capture total across both served gateway pipes, with the truncation
  notice paid for out of it.** #559 bounded each served pipe separately (4 MiB per stream) and
  added an 8 MiB ceiling across the pair, but the two bounds were only ever verified apart, and
  the interaction is the part that can go quietly wrong: a pipe cut by the *shared* budget trims
  its own notice to fit and charges it against already-retained bytes, and a bug there produces a
  capture that announces itself while silently omitting evidence. This change adds the tests for
  that combination. A served gateway that floods **both** pipes past the per-stream ceiling is now
  driven end to end: the pair the harness hands back never exceeds `MAX_TOTAL_CAPTURE_BYTES`, each
  cut pipe carries a truncation notice, and a gateway that stays inside every ceiling carries
  none — so "a notice is always present" cannot pass. The accounting is pinned at the unit level
  too, with the mutations named in #567 each shown to fail: charging the notice after the total is
  computed, dropping the trim-back, and charging the shared total against bytes that were never
  retained. The first of those is unobservable in the combined-flood shape alone and is caught by
  the per-stream form instead, which is why both shapes are kept. No served capture's bound
  changes: the ceilings were merged in #559; this only proves they hold together.

- **Split `context_control/membership.rs` so it stops hiding a 21-line hard-limit breach behind an
  exemption that denied it.** The file measured **421** nonblank lines against
  `rust_production_max = 400`. Its `policy.toml` exemption stated that count accurately and then
  asserted the implementation "remains below the hard limit" — false, and a claim the size gate
  was in a position to refute and did not, because `size_findings` skips an exempt path *before*
  reading it, so the prose was the only place the breach was recorded. The
  effective set, its per-reference outcomes, the dispatch projection, the revalidation state and
  the typed error vocabulary now live in a sibling `membership_effective.rs`, following the same
  `#[path]` idiom the module already used for `membership_resolution.rs` and
  `membership_selector.rs`, at **274** and **167** nonblank lines — both inside the 400-line hard
  limit, so the exemption is **deleted** rather than reworded. This is a pure source move: every
  item is re-exported from the parent so its public path, serde attributes and schema strings are
  unchanged, and no behaviour differs. `repo-policy --strict` reports one fewer exempted breach
  (3, down from 4; #606 took the pre-#606 count of 5 to 4), and `membership.rs` is gone from that
  list. No production, protocol, or runtime effect.
- **Split the provider-session metadata store's filesystem boundary into three sibling
  modules, so the file that hid a 57-line hard-limit breach is gone rather than reworded.** `state_store.rs` measured
  **457** nonblank lines against `rust_production_max` of **400**, and its `policy.toml` exemption
  asserted that 457 "remains below the hard limit" — false on the direction, and invisible to the
  size gate, because `size_findings` skips an exempt path before reading it. #569's exemption
  verifier is now able to refute exactly that sentence. The file carried two separable concerns:
  the **read and path-safety** rules (open without following a symlink, reject a path that escapes
  its directory or has a permissive mode), the **durable write** (write a uniquely named temporary
  file, give it private permissions, flush, then rename over the target), and the **refusal
  vocabulary**. Those move to `state_store_io.rs`, `atomic_replace.rs` and `store_error.rs` under
  the existing `state_store/` directory, beside `owner_lease.rs`, and the parent keeps the
  envelope, key and scope handling. The result is **272** / **110** / **68** / **49** nonblank
  lines — all inside both the 400 hard limit and the 300 preferred limit — and the exemption is
  **deleted**, not corrected: rewording the count instead of splitting the file is the exact
  failure this entry describes. This is a pure source move, checked mechanically against `main`:
  all **32** items the original file declared are present after the split, with none added and none
  dropped. No behaviour change — the `unix` and non-`unix` variants of each platform-sensitive
  helper move together, `owner_lease.rs` keeps its existing `super::` imports through a re-export,
  and the error enum is re-exported from the parent so `ProviderSessionMetadataStoreError` keeps
  its public path. Closes #571.

- **Make the management client's header allow-list guard non-vacuous, and correct the criteria
  that produced it.** The guard added for the `Accept` fix transcribed the gateway's
  `header_is_allowed` exactly, then added a nineteenth entry, `idempotency-key`, under the comment
  "Admitted by the gateway as a non-gateway header" — a category that does not exist, since
  `header_is_allowed` is a bare `matches!` with no exemption clause. Separately, the test only ever
  built a head with the idempotency argument `None`, and the header is emitted only in the `Some`
  branch, so the entry could neither fail nor be reached: the test asserted over a request that
  never carried the header it existed to police. Measured, that nineteenth entry was the sole
  difference between the guard passing and failing, so a head the gateway refuses with
  `400 unsupported_header` was asserted as safe. The fabricated entry and its false comment are
  removed; the plain head is now asserted against the gateway list outright, and the idempotent
  head is pinned to differ from that list by *exactly* `idempotency-key` — which fails both if a
  new unadmitted header appears and if the header is ever dropped. A new test asserts the guard is
  non-vacuous, which is the check whose absence let the defect through. The header is deliberately
  **not** removed: it is required by the management server (`idempotency_key_required`) and declared
  `required: true` on 16 operations across the memory and control OpenAPI contracts, so dropping it
  would turn 16 policy mutations into `400`s. The documented invariant is the precise one — every
  header this client sends is admitted by the server it is pointed at, which today is the harness
  management listener, not the gateway. Refs #598, #560.

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

- **Retire five unreachable Rust sources and gate the whole class.** `#491` found five tracked `.rs`
  files that no crate root reached, so they never compiled and their tests never ran. Four are
  superseded duplicates: `runtime_v3_episode_actions.rs` against the `include!`d
  `runtime_v3_episode_helpers.rs` (whose `retain_operation` is stricter, including the payload
  check), `runtime_v3_lifecycle_reconnect_test.rs` against the recovered reconnect test, and the
  `#220` residue `policy_owner/owner_impl.rs`/`change.rs`. The fifth, `sts2-astra-bridge_tests.rs`,
  held one assertion with no live counterpart, now ported into `sts2_astra_bridge_tests.rs`.
  `repo-policy` enforces `RUST002`: a tracked `.rs` inside a compiled crate that no crate root
  reaches through `mod`, `#[path]`, `#[cfg_attr(..., path = ...)]`, or `include!` now fails
  `--strict`, so a lost `mod` line turns a check red instead of silently dropping coverage. No
  runtime, provider, game, or native behavior changes. Closes #491.

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
