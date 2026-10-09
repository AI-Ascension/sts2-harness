# Changelog

All notable user-visible or operational changes to this project are documented here.

The project follows Semantic Versioning once versioned releases begin. Foundation work does not
claim a released harness version or runtime compatibility.

Completed entries that no longer fit the active file's preferred size budget are preserved verbatim
in [`docs/CHANGELOG-ARCHIVE.md`](docs/CHANGELOG-ARCHIVE.md) and the dated archives beside it,
including [`docs/CHANGELOG-ARCHIVE-2026-09-28.md`](docs/CHANGELOG-ARCHIVE-2026-09-28.md) and
[`docs/CHANGELOG-ARCHIVE-2026-10-08.md`](docs/CHANGELOG-ARCHIVE-2026-10-08.md).

## Unreleased

- **Opt-in map context gains durable invocation receipts.** Schema 9 adds a bounded,
  metadata-only table and preserves existing execution data when migrating schema 8.
  The runtime records intent before the map read and binds the validated response to the
  parser-accepted decision context before decision reuse or provider reservation. Unfinished
  receipts refuse another read after restart; non-map decisions retain approved-reference
  compatibility. Supplied-fact consistency does not attest authoritative owner provenance.
  This component does not complete the broader recipe, Studio or native acceptance gates.
  Refs #97; see [ADR 0086](docs/decisions/0086-durable-recipe-v2-map-invocation-receipts.md).

- **Exact-artifact publication adds the unprivileged held-descriptor fallback.** The held unnamed
  descriptor first uses the successful `AT_EMPTY_PATH` route; only `ENOENT` selects the verified
  procfs descriptor fallback. Ordinary absent reads remain `Missing`; write, link, and sync
  failures remain typed `Persistence`. Retained failure/history behavior and the 300/400/500
  source/test/docs caps stay intact. At this source-review increment, the candidate remains
  source-only and locked checks and corrected-path runtime qualification were NOT_RUN. No later
  gate result is implied; no native, provider, or game-compatibility claim. Refs #847.


- **Workflow seed support gains a v2 discovery catalog.** Authenticated clients can inspect structural durable support and readiness hints, while the additional structural mode gate runs after owner revalidation for new Missing submissions. The v1 catalog and v2 request shapes remain unchanged; source/component evidence does not establish native setup or seed settlement, deployment, or release. Refs #103.

- **Lifecycle process-recovery tests cover three synthetic crash cuts.** A child creates its initial owner and reaches the requested cut before termination; the parent reopens only afterward to check held or exact-result recovery without a second synthetic effect. The added cut stops after the possible-write journal and one fsynced synthetic effect attempt, before a handle or response receipt is delivered. This test-fixture correction does not change production behavior. Refs #142.

- **The served workflow consumes its admitted decision profile at Exo dispatch.** Enveloped mode advertises Available only for validated, inspected `decision.live.v1`; unpinned or raw-wire descriptors remain Unsupported. Explicit `STS2_EXO_ADMISSION=legacy` retains the previous no-catalog provider path and exposes no inspected profile catalog.
  The factory freezes that mode, refuses later mode changes, and binds the complete effective Exo settings before runtime creation; normal and prepared Enveloped dispatch recheck the exact revision before effects.
  Synthetic transport tests cover one decision profile, not paid-provider or native execution; planner execution and additional profiles remain unsupported. Refs #146, Studio #112.

- **The pinned Exo process oracle refuses explicit old-history input before model egress and covers the existing synthetic v2 mode.** It requires the unsupported history field containing `old-history-private-sentinel` to receive `exo_bridge_invalid_request` with zero loopback model requests. The v2 case checks the decision, receipt identities, and exact full-body model projection through the shipped bridge and pinned Exo process. The model and host remain synthetic; this is not live-provider or native continuity evidence. Refs #109.

- **Harness implements the context-owner T2 routes in source.** ADR 0080 preserves the 2026-10-04 T1 decision and records durable eligible-item, draft/revision, preview, mutation-receipt, and draft-publication operations on schema 3.
  Console #48 adds trusted subject-bound grant ingress (`461bd9b`); the production outbound owner adapter and encrypted durable exact-request mutation intent remain pending.
  Separate-process Console R3 response/recovery and the independent ADR 0021 sidecar trust join remain pending; this records source status, not acceptance.
  Refs #391, Console #18; native, provider, deployment, and production-host evidence remain separate.

- **The tautological default-sink probe is deleted.** `the_default_sink_reports_rather_than_discards`
  asserted a hard-coded `true`; a mutation that made `StderrFailurePort::report` a no-op still left
  **530 passed, 0 failed**. The `reports()` predicate and probe were removed. The first exact-write
  replacement in #826 used a process-wide writer hook, which the separate #829 follow-up below
  removes after source review found cross-test capture and panic-leak risks. Compatibility:
  diagnostics and test surface only. Refs #824, #819.

- **The default stderr proof now runs in an owned child process.** #829 removes the mutable
  process-wide writer override and restores the production `eprintln!` path. An exact helper-test
  child invokes `ManagementFailureSink::default()` and captures its real stderr to bounded output
  files. Two barrier-released children keep distinct concurrent report sets in their own stderr;
  another catches a synthetic panic and verifies a later default report still arrives. Every child
  is killed/reaped if its five-second wait expires or unwinds. A no-op final write fails the
  exact-line assertion; attached-port/server failure, attribution and redaction tests remain. This
  is a source-derived isolation correction, not a report of a CI flake or deployed incident.
  Production report text and behavior are unchanged. Refs #829, #826.

- **Every management request the owner serves is now reported on stderr, so an empty owner
  stderr is no longer ambiguous between a crash, a signal kill and a hang.** Studio's
  intermittent `submission_refused_502` had recurred eight times, and in every occurrence the
  decisive fact was that the owner process emitted nothing at all: `owner-stdout.log` was 0
  bytes, `owner-stderr.log` held 160 bytes that belonged to the Studio fixture rather than the
  owner, and `serve-workflow.log` was 0 bytes, while the gateway log proved the capture path
  itself was sound. That silence is what made acceptance criterion 3 undecidable — a crash, a
  kill and a clean-but-mute exit are indistinguishable in an empty log — so the management
  server now writes one bounded line per request on the same stderr the operator already
  captures. Each served request emits a `request_start` line with a UTC timestamp, epoch
  seconds, a per-server `request_id`, and an exact method-aware route template with dynamic
  identifiers replaced by placeholders. Every terminal line carries `elapsed_ms` from a
  process-local monotonic clock; `request_end` includes actual status, while `request_abandoned`
  records a typed code when the response could not be transmitted. A connection without a
  readable request cannot have a start marker, so
  its terminal marker is `request_unreadable`, which names its own condition. The contract is
  stated once, in `http_lifecycle.rs`: **a connection emits exactly one terminal line, and every
  terminal line is attributable — either it follows a start line that named the request, or it is
  itself named. The absence of a terminal line is the signal that the connection did not finish.**
  A stall or a panic on the request thread therefore surfaces as a start line with no terminal
  line, which is what makes those two states distinguishable from a request that finished
  normally.

  That `request_unreadable` marker exists because driving the real binary found the defect
  it fixes. A peer that connected, sent a partial request, and then went silent hit the read
  budget and emitted a bare `request_end` with no start line before it and no route anywhere on
  it — a terminal line nobody could act on, which is the same unattributed-report defect class
  #819 documents for the pre-existing `eprintln!`, reproduced by the new code. The error is
  still returned unchanged and the peer still receives the same `400`; only the marker differs.

  Three properties keep the log safe. The query string is dropped, and a route is logged only
  when its method and full shape match a served template; dynamic IDs become placeholders, while
  every other path receives the fixed `route=unmatched` label. Credentials and absolute local
  paths are therefore unrecoverable from the line. Labels remain byte-capped and single-line,
  and non-printable bytes are replaced. Abandonment reports the typed code only, never
  `HttpError::message`, which can embed caller bytes from the `io::Error` text.

  Two placement decisions carry the reasoning. The start marker is emitted *after* the request
  parses, never before, because until the read succeeds there is no method and no route, and a
  start line naming a route the server never received would be a lie in the one log that exists
  to be trustworthy. The terminal marker keys on whether the response was *transmitted*, not on
  whether it was *computed*: a delivered 4xx is `request_end`, and a computed response the peer
  reset away is `request_abandoned`. Cost is bounded at two lines per request with no per-byte
  work, so the Studio submission path gains no meaningful latency.

  This work coordinates with #819 rather than competing with it. #819 (PR #821) reports an
  undeliverable response through the assertable `ManagementFailureSink` port, naming the peer,
  the route and the cause; this change preserves that exact return contract and adds the
  per-request markers alongside it, so one event is described at two resolutions — #819 at the
  connection, #820 per request — and neither introduces a second report mechanism for the same
  failure. Observability is proven by mutation in both directions: deleting the start emission,
  disabling the terminal-marker block, reporting abandonment as an ordinary completion, and
  disabling the route-vocabulary filter each turn the suite red, and the secret-absence,
  absolute-path-absence, log-forging and unknown-route tests are among what goes red.
  Compatibility: diagnostics only; the status codes, error classes, response bounds, per-phase
  deadline and every delivered response are unchanged, and no timeout or retry was added.
  Refs #820, #819.

- **An undeliverable management response is now reported through an assertable port, and the
  report names the peer and the route it failed to answer.** #817 stopped discarding a management
  response the owner could not transmit and reported it with an `eprintln!`, but nothing asserted
  that line: deleting it and restoring `let _ = handle_connection(...)` passed all 524 lib tests,
  verified twice by independent review, because the report was write-only to a process-wide stream
  and no gate could see it. Observability that is hard to unit-test does not get skipped, it gets a
  port. `ManagementFailurePort`/`ManagementFailureSink` follow the repository's existing
  `BoundaryCaptureSink` convention and default to the same stderr report #816 added, so production
  behaviour is unchanged; a test attaches a recording port and asserts the report. Deliberately no
  `disabled()` constructor, because an inert default would restore exactly the silence this port
  exists to remove, and would do so by one unremarkable builder call. Attribution names the peer
  address — which `run_server_loop` had in hand and discarded as `_peer` — the `METHOD path` read
  before dispatch consumes the request, and the owner's own `cause` code. A request that never
  parsed is reported as `(not read)` rather than with a guessed route, because a guessed route is
  worse than none: it would be believed. Every field is escaped and bounded to 256 characters
  following `escape_bounded`, so a peer-supplied route cannot inject a second operator line or
  flood the log; no field can carry a credential, environment value, or local path. Compatibility:
  diagnostics and test surface only; the response bound, the per-phase deadline, the status
  codes, and every delivered response are unchanged. Refs #819, #816.

- **Every management request the owner serves is now reported on stderr, so an empty owner
  stderr is no longer ambiguous between a crash, a signal kill and a hang.** Studio's
  intermittent `submission_refused_502` had recurred eight times, and in every occurrence the
  decisive fact was that the owner process emitted nothing at all: `owner-stdout.log` was 0
  bytes, `owner-stderr.log` held 160 bytes that belonged to the Studio fixture rather than the
  owner, and `serve-workflow.log` was 0 bytes, while the gateway log proved the capture path
  itself was sound. That silence is what made acceptance criterion 3 undecidable — a crash, a
  kill and a clean-but-mute exit are indistinguishable in an empty log — so the management
  server now writes one bounded line per request on the same stderr the operator already
  captures. Each served request emits a `request_start` line carrying a UTC timestamp, epoch
  seconds, a monotonic per-server `request_id`, the method and a bounded route, and then exactly
  one terminal line: `request_end` with the status the peer was actually told and the elapsed
  time, or `request_abandoned` with a typed code when the response was computed but could not be
  transmitted. The contract is stated once, in `http_lifecycle.rs`: **a request emits a start line
  and exactly one terminal line, and the absence of the terminal line is itself the signal that it
  did not finish.** A stall or a panic on the request thread therefore surfaces as a start line
  with no terminal line, which is what makes those two states distinguishable from a request that
  finished normally.

  Three properties keep the log safe, because the harness is a security boundary and a logging
  change that leaks a token is worse than no logging change at all. The query string is dropped.
  Route segments are checked against a closed vocabulary of the routes the server actually
  serves, so an unserved segment becomes `/?`: the separator keeps the segment *count*, so the
  line stays diagnosable, while a credential pasted into a path and any absolute local path —
  which always contains segments outside the list — are unrecoverable from the line. Labels are
  byte-capped and non-printable bytes are replaced, so a caller can neither drive unbounded log
  volume nor forge a second operator line. Abandonment reports the typed code only, never
  `HttpError::message`, because `io_http_error` builds its message from an `io::Error` whose text
  can embed caller bytes.

  Two placement decisions carry the reasoning. The start marker is emitted *after* the request
  parses, never before, because until the read succeeds there is no method and no route, and a
  start line naming a route the server never received would be a lie in the one log that exists
  to be trustworthy. The terminal marker keys on whether the response was *transmitted*, not on
  whether it was *computed*: a delivered 4xx is `request_end`, and a computed response the peer
  reset away is `request_abandoned`. Cost is bounded at two lines per request with no per-byte
  work, so the Studio submission path gains no meaningful latency.

  This work coordinates with #819 rather than competing with it. #819 (PR #821) reports an
  undeliverable response through the assertable `ManagementFailureSink` port, naming the peer,
  the route and the cause; this change preserves that exact return contract and adds the
  per-request markers alongside it, so one event is described at two resolutions — #819 at the
  connection, #820 per request — and neither introduces a second report mechanism for the same
  failure. Observability is proven by mutation in both directions: deleting the start emission,
  disabling the terminal-marker block, reporting abandonment as an ordinary completion, and
  disabling the route-vocabulary filter each turn the suite red, and the secret-absence,
  absolute-path-absence, log-forging and unknown-route tests are among what goes red.

  Two passes over the merged shape found ways the contract did not hold, and both were
  reproduced against the real owner binary rather than only against the suite. A connection
  that ended without a readable request emitted a bare `request_end status=400` with no
  start line before it and no route anywhere on it, which is unattributable — so that case
  now emits `request_unreadable` naming its own condition and the typed code that stopped
  the read, because a connection with no readable request cannot honestly have a start
  marker. Separately, a panic inside `dispatch` unwound the worker after `request_start` had
  been written, so a crash and a hang both left a start line with no terminal line — the
  exact distinction this log exists to make, lost to the one case it could not cover.
  `dispatch` is now unwound: a panicking handler emits a `request_panicked` terminal marker
  with the harness-owned `code=request_panicked` before the unwind is resumed, so the fault
  is reported as itself and still fails the connection loudly instead of being swallowed
  into an ordinary error response. Neither marker reads an `io::Error` message or a panic
  payload, so the redaction rule holds for both.

  Review of the merged shape found one way the contract did not hold. A panic inside
  `dispatch` unwound the connection worker after `request_start` had already been written,
  so a crash and a hang both left a start line with no terminal line — the exact
  distinction this log exists to make, lost to the one case it could not cover. `dispatch`
  is now unwrapped: a panicking handler emits a `request_panicked` terminal marker with the
  harness-owned `code=request_panicked` before the unwind is resumed, so the fault is
  reported as itself and still fails the connection loudly instead of being swallowed into
  an ordinary error response. No panic payload is read, so the redaction rule holds. A
  determinate panicking `Authenticator` proves it over a real socket; deleting the emission
  turns that test red.
  Compatibility: diagnostics only; the status codes, error classes, response bounds, per-phase
  deadline and every delivered response are unchanged, and no timeout or retry was added.
  Refs #820, #819.

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
