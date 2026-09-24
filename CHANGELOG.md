# Changelog

All notable user-visible or operational changes to this project are documented here.

The project follows Semantic Versioning once versioned releases begin. Foundation work does not
claim a released harness version or runtime compatibility.

Completed entries that no longer fit the active file's preferred size budget are preserved verbatim
in [`docs/CHANGELOG-ARCHIVE.md`](docs/CHANGELOG-ARCHIVE.md).

## Unreleased

- **Admit a bounded pre-agent read-only recipe.** A new `recipe` module fixes the source-only
  contract behind #97: an authored workflow may declare a bounded, versioned recipe of approved
  read-only tool reads that the harness admits before provider dispatch, with a fixed refusal order,
  a declared topological step order with no cycles or forward references, and mutation tools refused
  from the read-only catalog ([ADR 0073](docs/decisions/0073-pre-agent-read-only-recipe-admission.md)).
  Collection execution, provenance and the Studio round-trip remain open. Refs #97.

- **Plan, schedule and compare bounded same-start branch experiments.** A new
  `benchmark_manifest::branch_experiment` module fixes the effect-free contract behind issue #119: a
  versioned declaration of one verified fork point, a fork strategy, child policies and per-child and
  total budgets; a stable per-child trial key with its own fresh provider/context namespace; a
  same-start admission re-check that keeps a prefix-only start out of exact-restore statistics; a
  retry-safe recorded scheduler that reconciles a lost reply without double-scoring a trial; an
  aligned comparison that separates declared policy divergence from restore failure and does not let
  an identical endpoint erase an earlier divergence; and a sanitized report carrying a keyed handle
  and no exact digest. Source-only
  ([ADR 0072](docs/decisions/0072-branch-experiment-comparison.md)): the live children, the restore
  and the provider calls stay with the gateway and game-mod. Refs #119.

- **Resolve and durably bind an authored workflow's seed.** A new `seed_binding` module fixes the
  source-only contract behind #103: an explicit or generate-once seed normalizes to one bounded
  canonical UTF-8 form, a generate-once run draws exactly once and reuses the persisted effective
  seed across duplicate requests, lost responses and restarts, the effective seed is persisted before
  any setup mutation (failing closed), and a wrong instance, stale baseline or lease, unsupported
  setup, or conflicting persisted seed is refused before any draw. A recording transport proves the
  persisted effective seed and operation identity are sent unchanged. Native seed acceptance stays
  gated by sts2-game-mod#79
  ([ADR 0071](docs/decisions/0071-authored-seed-binding.md)). Refs #103.

- **Bind a benchmark rerun admission to the exact declaration it compared equal.** `RerunAdmission`
  now owns the admitted `Manifest`, reachable only through `RerunAdmission::declaration()`, so a
  `RerunAllocationSeam` cannot allocate for a declaration other than the one whose controlled inputs
  compared equal. Source-only contract tightening for #121; the equal path is unchanged.
  Refs #121.

- **Make the served capture surface configured and fail-closed.** Which sink the served composition
  attaches and what it retains is now an owner decision recorded in
  [ADR 0070](docs/decisions/0070-served-capture-configuration-and-retention.md): an unset surface
  keeps the merged in-memory recording ring, `metadata` and `off` are selectable, and every
  contradictory or out-of-range `STS2_WORKFLOW_CAPTURE_*` value is refused at startup rather than
  silently downgraded. Restart-durable capture bytes and the unrecorded Ollama `HttpBody` boundary
  remain accountable residuals (#145). Compatibility: no change to the served default behaviour.
  Refs #398.

- **Admit alternative gameplay forks from verified seeded replay prefixes.** A new
  `benchmark_manifest::prefix_fork` module fixes the effect-free fork-admission contract behind
  #117: an exact seed/profile/build/compatibility binding, a settled nonterminal boundary with
  complete receipts and one resolved legal action, a zero-provider-call replay, bounded sibling
  forks with distinct identities, and a forward-only replay-to-child handoff that reconciles a lost
  target. Source-only ([ADR 0069](docs/decisions/0069-prefix-fork-admission.md)). Refs #117.

- **Orchestrate isolated cold-launch benchmark trials from one pristine baseline.** A new
  `benchmark_manifest::cold_launch` module fixes the per-trial isolation contract behind issue #122:
  an immutable baseline binding the artifact digest, launch profile and closed telemetry exclusions;
  an exclusive, bounded destination lease; an opaque gateway-attested process birth with its own
  instance generation, since a PID can be reused; a readiness proof bound to that birth; a recorded
  stage machine that reconciles a lost reply without adopting another trial's state and quarantines
  an uncertain destination; and machine-readable cold-start evidence whose cleanup failure is
  distinct from the gameplay outcome. Source-only: native process evidence and the real child-process
  lane stay gated by sts2-game-mod#79
  ([ADR 0068](docs/decisions/0068-cold-launch-trial-isolation.md)). Refs #122.

- **Plan, schedule and report reproducible multi-policy benchmark suites.** A new
  `benchmark_manifest::suite` module freezes an ordered seed corpus, policy axis, repetition count,
  evaluator revision, declared budgets and predeclared metrics under a versioned manifest; plans one
  stable logical trial per suite revision/case/policy/repetition with its own provider/context
  namespace; keeps retry-safe attempt lineage so a replayed settlement is idempotent and a conflicting
  one is refused; preserves attempt counts across resume; and exports a sanitized aggregate with
  explicit denominators, honest paired comparisons and metric availability, never counting an
  infrastructure failure as a defeat, an unavailable cost as zero, or an unverified start inside an
  exact-start group. Source-only: native exact-start certification stays gated by #126
  ([ADR 0067](docs/decisions/0067-reproducible-benchmark-suite-scheduling-and-reports.md)). Refs #125.

- **Add offline trace-bundle admission and a bounded reproducer for divergence diagnosis.** A new
  `trace_divergence` module derives an immutable `TraceBundleManifest` per bundle, admits two bundles
  by closure, profile and action-schema coverage *before* comparing, compares bounded record views,
  reports explicit record/entry/byte truncation, and exports a `ReproducerPrefix` that replays only
  up to the failing boundary and validates against the original source. Offline and read-only; the
  public status stays digest-free. Native mismatch validation remains gated by #123
  ([ADR 0066](docs/decisions/0066-offline-trace-bundle-admission-and-reproducer.md)). Refs #124.

- **Record the two source-only Jev-runner decisions.** The wall-clock-sensitive global-time-budget
  test's fixture strategy is recorded in
  [ADR 0063](docs/decisions/0063-jev-runner-first-arm-admission.md): the first scheduled arm is
  admitted structurally rather than by fixture timing (#388). The 1,000 ms teardown cleanup bound is
  accepted as a host-load-dependent contract in
  [ADR 0064](docs/decisions/0064-jev-runner-teardown-cleanup-bound.md), with the strict closure
  assertion intact and the sampling limitations retained (#394). The Jev-evaluation Node suite was
  rerun without retry masking; first-attempt results are in
  [the no-retry matrix evidence](docs/evidence/jev-evaluation-noretry-matrix-20260923.md).
  Compatibility: documentation only; no code, record shape, or runner contract change.
  Refs #388, #394.

- **Admit the host-offered `continue_run` action in the runtime-v3 path.** A host that offered
  `continue_run` beside `start_run` failed the whole observation: the production runtime-v3 parser
  and the fair-play sanitizer both refused any action kind outside their allowlists, and the parser's
  kind table fell through to `save_quit`. Both boundaries now admit the host-owned identity with an
  optional `run_id` discriminator (`{"kind":"continue_run"}` or
  `{"kind":"continue_run","run_id":"profile1"}`), preserve the host-generated `action_id`, and still
  refuse a save path, a null or non-identity `run_id`, an unknown kind and an extra field. The
  allowlist is now the single source of truth for a kind's field contract and its typed action, so an
  unknown kind is rejected before dispatch instead of being coerced into another action, and the
  continuation is bound to the offered generation so a stale catalog is refused before any effect.
  Refs #390.
- **Freeze the host-offered `continue_run` admission contract and prove its consumer-first
  boundary.** The two accepted shapes and the refusal list are now recorded beside the runtime-v3
  admission (`payload_contract`), the Exo projection (`schema.rs`) and `docs/ARCHITECTURE.md`,
  citing `sts2-harness#415` (`551ec19d`) and `sts2-game-mod#210` (`8a655143`); focused tests cover
  the valid offer and the malformed, unknown-field, stale, foreign-profile and unoffered refusals. Refs #390.
- **Carry the served managed-boundary receipt ledger across a process restart.** A restarted served
  composition rebuilt an empty in-memory ledger and wrote an accepted boundary a second time. The
  receipt ledger now has a versioned durable image, an owner-supplied port (`with_dispatch_ledger_port`)
  and a file-backed store the served binary attaches when `STS2_WORKFLOW_DISPATCH_LEDGER` names a path:
  a restart reloads the receipts and refuses a second write or an unreadable store, and unset receipts stay session-lifetime. Refs #108, #94.
- **Execute the shipped host-lease campaign downstream in the runtime peer contract lane.** The
  `the_env_configured_campaign_downstream_answers_a_signed_install` witness was declared
  operator-only and no step invoked it, so nothing in CI proved that the *environment-configured*
  long-lived `synthetic_mod_server` process advertises `host_lease=enabled` and terminates a signed
  `lease_install_request` — the half of the sideband a long campaign depends on, while only the
  in-process terminal was covered. The lane now builds that operator target, points
  `STS2_SYNTHETIC_MOD_SERVER_BINARY` at it, and runs the witness, and the fail-closed lane check
  pairs each lane with the operator marker its own source declares. Refs #94.
- Verify **the System One bridge's refusals at its own process boundary**, and pin the tie it does not
  resolve. Three acceptance criteria were carried unverified because nothing exercised the seam they
  named. A shell transport now drives the real `sts2-jev-bridge`: a non-`200` exit, a raw HTTP status
  line read as a body, a malformed envelope, an out-of-catalog choice, a well-formed answer past the
  128 KiB bound, and a transport failure are each refused with the bridge's exit status and no
  decision, and every case proves the provider answer arrived first, so a case whose transport never
  ran cannot pass. The operator credential is asserted both positively — `TYPESAFE_API_KEY` reaches
  the transport by name — and negatively, in a captured request, a record, `--describe` and a
  refusal. A local provider declaration's digest pin is exercised at the runtime boundary with a
  control that pins the digest those bytes really have, so the refusal is the mismatch and not the
  presence of a declared bridge. And two equally likely options are shown to be resolved by nothing:
  the answer's own `choice` is returned, the gate is applied to the confidence the provider stated,
  and the tied identifier named in the bridge-authored rationale is the one the sorted probability map
  orders last, so the same answer produces the same rationale whatever order the provider wrote its
  keys in. Refs #284, #285, #288.
- **Route the executable REST selector composition into the runtime peer contract lane.** The
  `runtime_v4_rest_executable_composition` witnesses (issue #148) assert the authored
  observe → decide → execute-action → terminal graph settles each durable REST receipt before it
  advances, but no gate invoked them, so the regressions they pin were as invisible as if they had
  never been written. The lane now runs both against the pinned gateway and MCP peers, each with
  its own evidence directory so a REST run cannot overwrite the generic composition's
  `result.json`. The peer-binary environment those steps repeated moves to one job-level `env:`
  block, which keeps the workflow inside its nonblank-line budget after the addition, and the
  fail-closed lane check now covers both composition binaries. Refs #148.
- **Execute the idle-adoption policy-rebind regression in the runtime peer contract lane.** The
  `served_decision_survives_changed_policy_adopted_while_idle` witness was written for the permanent
  mid-run policy-adoption fence (issue #255) with the same `#[ignore]` operator marker as its
  siblings, but no lane step invoked it, so the regression was as invisible as if it had never been
  written. The served policy step now also runs it against the pinned gateway and MCP peers, and a
  fail-closed lane check rejects a declared operator-only composition test that no lane step executes
  or a lane `--exact` invocation that names no declared test. Refs #255.
- **Record the served managed boundary before it writes.** The exact material a served managed
  decision approved was never compared with the bytes it wrote. The exchange now runs inside the
  recording write port, so a session with no recording sink refuses (`prepared_boundary_unsupported`)
  instead of publishing exactness, and the served composition attaches a bounded recording ring so a
  managed decision records its boundary rather than refusing. See
  [ADR 0061](docs/decisions/0061-served-managed-boundary-recording.md). Refs #108.
- **Size the jev process-teardown pipe-cleanup bound above host-load jitter.** The paired runner gave
  a killed process group 250 ms to close an inherited pipe and reported `child_closed: false` past
  that, but a clean host's kill-to-close tail already reaches 250-306 ms under load, so the flag read
  "closure unconfirmed" for a process that closed a millisecond later and the offline runner-process
  contract test flaked. The grace is now a documented 1000 ms contract value, and a new escaped-
  session control proves the flag stays false when a bound is genuinely spent, so the assertion was
  strengthened rather than relaxed. Compatibility: none; the flag's meaning is unchanged. Refs #394.

- **Let the jev execution budget govern arm admission, not filesystem timing.** The paired runner
  re-checked the budget after reserving an arm, so a slow filesystem cancelled an admitted first arm
  and made the offline global-time-budget contract test fail, with a re-run masking that red. An
  admitted arm now launches its child bounded by the smaller of the two budgets. Refs #388.
- **Admit a live episode from the provider lane's declared capability, not from a name.** A live
  `STS2_LIVE_EPISODE=true` run was admitted only when `STS2_PROVIDER_KIND` was exactly
  `openai-astra`, which left the Exo lane unable to be admitted for one, while any unimplemented name
  fell through the non-bridge branch and ran under the reviewed Exo source revision. The kind is now
  a type whose declarations decide whether the lane is a locally launched bridge and whether it
  claims live-episode capability (`openai-astra` and `exo`); an unimplemented name is refused while
  settings are assembled, and `exo` carries a live episode only under `STS2_EXO_ADMISSION=envelope`.
  The admitted mode is installed once and is what the replay stream and the live diagnostics read.
  Compatibility: the documented lanes are unchanged; a `STS2_PROVIDER_KIND` no lane implements is
  now refused instead of running silently as the reviewed executor. See
  [ADR 0060](docs/decisions/0060-live-episode-capability-admission.md). Refs #145.
- Add Linux [Jev streaming mode](experiments/jev-plays-sts2/STREAMING.md): retain the game with manual resume; preserve timed benchmarks. Automatic terminal progression remains unavailable.
- **Hold one live decision attempt so a lost reply cannot buy a second one.** A live `Decide` node
  took a fresh `ModelExecutionId` on every entry and kept no record of the attempt, so an
  `Unresolved` refusal left the run `NeedsOperator` with `pending_operation: null` and the next
  `Step` paid the provider again. The attempt is now installed and durably recorded before
  `decide_for`, released only by a refusal the provider owner reported before it could write, and
  re-used by a retry that reproduces the admitted request digest. Compatibility: additive; no wire or durable record changes. See [ADR 0059](docs/decisions/0059-held-live-decision-attempt.md). Refs #108.

- Run the existing compiled Jev paired-replay and frozen-pilot tests in both Node CI checks
  through a locked-build [entrypoint](experiments/jev-evaluation/compiled-ci.sh). Failures do not
  silently skip coverage. The transport stays synthetic; live gameplay benefit remains unverified.

- **Enforce the prepared-input token-measurement invariant on the read path.** `TokenMeasurement`
  claimed that `tokens` is `None` exactly when the provenance is `Unavailable`, but its fields were
  public and its derived deserializer accepted any shape, so a record claiming an absent provenance
  beside a byte count deserialized and `PreparedInputBudget::tokens()` reported that byte count as a
  token count. The fields are now private behind read accessors, and deserialization re-validates
  exactly what the constructors validate: `Unavailable` with a quantity, a non-`Unavailable`
  provenance with no quantity, `tokens == 0` and an invalid method are rejected rather than read
  back as a measurement. The Unicode eviction test now discriminates byte accounting from character
  accounting. No durable record, published schema, or consumer pin changes. Refs #381.

- **Refuse a served assembled input that does not fit beside its advertised output reserve.** The
  pre-existing `max_context_bytes` check bounded the request bytes alone; there was no served bound
  over the whole bytes actually sent. `ContextRenderLimits` and the context-owner descriptor both
  gain an optional `output_reserve_bytes`: `None` is exactly the prior contract, where response
  capacity stays bounded by the provider configuration, and a published reserve makes
  `max_context_bytes` the combined whole-input bound. The served managed decision then admits the
  assembled provider bytes against that bound before any dispatch, refusing
  `context_whole_input_budget_exceeded` and an unusable advertised reserve with
  `context_whole_input_budget_invalid`. Compatibility: additive; the field is optional and
  skip-serialized, so a descriptor that does not advertise it serializes byte-identically. See
  [ADR 0058](docs/decisions/0058-served-whole-input-output-reserve.md). Refs #107.

- Add a read-only [frozen Jev pilot profile](experiments/jev-evaluation/PILOT.md): ten pairs,
  twenty reserved attempts, exact-manifest reconciliation, per-arm refusal/gate diagnostics and
  matched input-token/latency accounting. No policy change or live gameplay benefit is claimed.

- Add an explicitly approved [paired Jev replay runner](experiments/jev-evaluation/RUNNER.md):
  pinned matching inputs, reserved budgets, independent redacted captures, bounded Unix processes
  and read-only recovery. No game action is dispatched; native/provider benefit remains unverified.

- Add opt-in Jev `--audit-dir` metadata sidecars with bounded, create-only Unix reservations,
  separate execution/input fingerprints, no raw prompts or action IDs, and no extra provider calls.
  Runtime stdout stays one decision; storage failures refuse it. Add a redacted paired reader and
  CI for the offline evaluation tests. Windows capture, native gameplay benefit and live paired
  orchestration remain unverified. See [capture documentation](experiments/jev-evaluation/CAPTURE.md).

- Let a provider ask **bounded semantic history through one harness-owned agent tool**. The history
  a run records was queryable inside the harness but not through the boundary an agent actually
  drives, so nothing could ask what happened without reaching around that boundary. The tool
  vocabulary is closed to the branch, kind, origin, subject, episode, sequence, limit and
  continuation axes — a question naming a path, bucket, artifact, record ordinal, offset, owner, run
  or epoch is refused rather than read — so the MCP game adapter cannot bypass the owned port to
  arbitrary artifact storage or reverse-call the harness. Selecting history is additive over the
  bootstrap profile: the advertised schema, tool set and digest widen to include the new tool, no
  shipped tool or inherited authority axis changes, and each additive turn keeps its own wire pin, so
  a relay holding an earlier profile's pin cannot relabel a frame into a history question and a
  bootstrap turn is refused on the pin history added. History is served only from the store the owner
  attached, that grant is re-checked on every read, and a session with no attachment refuses by
  capability name rather than answering an empty history; an episode travels per event, so the
  question cannot name one as a session axis. One answer must fit one feedback envelope, and an
  archive replay of a history turn diverges rather than being presented as a replayed read.
  Compatibility: additive — the v1 and v2 advertisements, pins and tool descriptions stay
  byte-compatible, the profile is opt-in, and no durable record or published schema changes. See
  [ADR 0057](docs/decisions/0057-harness-semantic-history.md). Refs #128.

- Record **queryable semantic combat and run history with causal provenance**. The host's bounded
  semantic event vocabulary had no harness-owned durable history behind it, so a run could not be
  asked what happened or why a value changed. The new `semantic_history` module appends each event
  against one run, branch, episode and authority epoch on a strictly advancing sequence, and carries
  both ends of an event — an actor and a target, each in the namespace it was minted in — plus the
  content a card play, pile move, purchase or offer names. A detail the kind requires and the host
  omitted is refused rather than stored as absent, as is a capture gap closed with an invented
  event; a causal parent is admitted only when the host stated one, in the same branch and epoch and
  strictly before its child, and an imported event never states one. An identical re-append replays
  and writes nothing while different content under one identity is a conflict; retention redacts a
  value in place rather than deleting the event or zeroing it. History is readable only through the
  harness-owned port, which re-checks owner and epoch on every read and refuses a caller naming

  A run's history is written out and read back as one document, and restoration re-derives
  every rule the append path applies — each record's shape, the epoch and coverage it claims,
  its stated parent, its recomputed digest and the branch lineage — so a truncated, reordered
  or edited document is refused rather than loaded as a history this boundary never wrote.
  An advance to a new authority epoch, which restarts host sequencing honestly, survives that
  restart with the sequencing expectation of the epoch now in force.
  Native saved history is backfilled through the same owned port and only as opaque bytes, so an
  importer cannot state a window, scope, epoch or cause the harness would then trust: a batch that
  names another scope, epoch or branch, an unknown member, another schema or a non-opaque identity
  is refused, an event that claims a native origin is refused rather than stamped as imported,
  imported records keep the coverage and source label they were captured under, and a batch that
  fails partway writes nothing at all. A granted reader can now spend a page's continuation and ask
  for a bounded causal explanation.
  storage directly; see [ADR 0057](docs/decisions/0057-harness-semantic-history.md). Refs #128.

- Name the **host's recovery reason** in an episode failure instead of reporting every recovery
  condition with one sentence. A runtime-v3 recovery state carries the condition that produced it
  in the sibling `code`, but the parser read only the stage, so a refused launch contract
  (`launch_contract_refused_<reason>`), an unconfigured host and an unavailable observation all
  arrived as `episode requires recovery before policy can continue`. The code is now bound to the
  observation and named in the failure when the host named one, and the established sentence is
  unchanged when it named none. It is held to the same identity rule as `state_id`, and only a
  recovery observation may carry one, so a reason the host names later needs no second change here
  while a code the schema cannot carry still fails closed. The same reason now survives the expert
  runtime profiles, whose observation is composed from the expert projection rather than carried
  over from the runtime-v3 read, so `runtime-v4-expert` and `runtime-v4-expert-rest-action` name it
  too. Compatibility: `breaking` for
  `EpisodeRunnerError::RecoveryRequired`, now a struct variant with an optional `code`; no wire
  field, schema or durable record changes, and the directory diagnostic stays in `game.log`
  unread. See [ADR 0056](docs/decisions/0056-harness-recovery-reason-token.md). Refs #355.

- Admit the **refused-launch-contract recovery code** on the legal-action read. The game-mod answers
  a refused launch contract with `503 launch_contract_refused`, or the prefix, `_`, and one bounded
  reason token, while the adapter admitted only `stale_generation`, `host_not_configured`, and
  `host_observation_unavailable`, so a refusal stayed fatal instead of becoming the bounded
  reobservation it names. The admitted set is now the producer's own rule rather than a second list:
  the bare prefix, or the prefix, `_`, and a token of 1 to 64 ASCII alphanumerics, `_`, or `-`. A
  code the mod cannot compose — a trailing separator, a dot, a slash, a space, a non-ASCII byte, a
  65-byte token, or a neighbouring string that merely starts the same way — still fails closed, as do
  other statuses, extra fields, and mismatched correlation. This mirrors
  `AI-Ascension/sts2-gateway#85`; the MCP consumer is a separate change and the native recovered
  screen transition remains unverified.

- **Report why a supervised child produced no answer at every seam that supervises one.** The
  provider transport no longer sends its child's standard error to the null device, but the two seams
  beside it, the one-shot lifecycle effect and the long-lived lookup supervisor, still discarded it,
  so a bridge that could not start, a bridge that exited with a status and a bridge that never
  answered all arrived as the same `Unavailable` or `Transport` and an outage they were not. The
  three seams now share one diagnostic boundary: a child the harness could not start is reported with
  the executable and the operating system's own message, a child that stopped by itself with its exit
  status and a bounded, escaped tail of what it wrote, and a child the harness had to stop -- a
  cancelled turn, an expired deadline -- only when it left something on the stream. The duplex
  supervisor drains that stream while its child runs, because a bridge that writes to a pipe nobody
  reads blocks on it while the protocol loop waits for its response. Nothing free-form enters a
  record, an error value, or a wire shape. Compatibility: a one-shot bridge that writes more standard
  error than the pipe holds now waits on that pipe, bounded by the exchange's deadline. Refs #352.
  Partial progress on #79 only.

- **Report why a provider transport did not start, instead of calling it an outage.** The transport
  spawned the bridge with the child's standard error sent to the null device, so a bridge that could
  not start and a provider that could not be reached arrived as the same `Unavailable`, the same
  `transport_unavailable` capture reason and a provider reservation whose failure class was `outage`.
  The bridge that produced that class here exits `9009` or `0x8009001d` before it reads its request
  and says why on standard error, which was the one channel naming the cause and the one discarded.
  That stream is now piped, and a failed exchange writes one operator line to the harness's own
  standard error, which the runtime already collects as `harness.err.log`: the exit status and a
  bounded, escaped tail of the child's message. Nothing free-form enters a record, an error value, a
  wire shape or the episode vocabulary, and a completed exchange reports nothing. Compatibility: a
  bridge that writes more standard error than the pipe holds now waits on that pipe rather than the
  null device, bounded by the exchange's deadline. Refs #348. Partial progress on #79 only.

- **Stop the Windows transport marking its exchange record.** `systemone_transport.ps1` appends to
  the file `JEV_CONTEXT_LOG` names with `[Text.Encoding]::UTF8`, which is a `UTF8Encoding` with its
  identifier turned on, and `AppendAllText` writes that preamble when it creates the file -- so the
  record begins with three bytes in front of the first record. The file is a JSON Lines stream read
  line by line, so a reader that opens it as UTF-8 cannot parse the one line that carries the state,
  the instructions and every option the model was given. The append now hands the call a
  `UTF8Encoding($false)`. The test
  `crates/harness/tests/jev_loop_windows_transport_record_mark_free.rs` scans the transport and fails
  if a mark-emitting encoding, cmdlet or constructor comes back. Refs #173. Partial progress on #79
  only.

- **Give the Windows lane's provider transport the environment it needs to start.** The runtime
  spawns the Exo bridge with the environment cleared but for `STS2_EXO_INHERITED_ENV_JSON`, and the
  bridge hands its own environment to the transport it spawns, so that list is the transport's whole
  environment. On this lane the transport is a `.cmd` that runs Windows PowerShell, and the list
  named only the credential and the recording path. Without `PATH` the command interpreter cannot
  resolve `powershell.exe` at all and the transport exits 9009; with `PATH` but without `SystemRoot`
  it resolves the interpreter and the interpreter cannot load its own managed assemblies
  (`0x8009001d`). Both were measured on the guest, where the same request to the same endpoint from
  the same host authenticated and answered, so the episode that ended as `provider is unavailable`
  had a transport that never started rather than a provider that was down. The lane now declares
  `PATH` and `SystemRoot` alongside the two names it already declared. The test
  `crates/harness/tests/jev_loop_windows_transport_environment.rs` scans the real script and fails if
  a name the transport needs is dropped, a name it does not need is added, or the list is declared
  twice, which is how a second declaration would silently replace the first. Refs #173. Partial
  progress on #79 only.

- **Write the Windows lane's shared files without a byte-order mark.** Windows PowerShell 5.1
  spells `-Encoding UTF8` as UTF-8 *with* a mark, and the lane wrote `override.cfg` that way. The
  game does not honour a marked override, so it resolved the shared default user directory, the mod
  compared that against the directory the lane had declared in `STS2_LIVE_USER_DIR`, and the episode
  ended with `live demo requires its isolated user directory` and no listener. Of the 212 episodes
  the guest still holds, the 18 the lane drove all resolved the default directory, while the 167
  whose `override.cfg` came from a writer that emits no mark all resolved their own per-episode
  directory. `override.cfg`, the seeded `settings.save`, and `authorization.json` now go through one
  `Write-TextFile` helper that constructs `UTF8Encoding($false)`; the PID and outcome files keep
  bare `Set-Content`, which writes no mark. Refs #173.

- **File the System One decision the bridge actually emits, and let the bridge emit the record
  itself.** `map_decision` returns four fields for a re-observation, but the filed `bridge_decision`
  carried only the rationale and the decision name, so the evidence artifact was still not the
  bridge's output even after its numbers had been corrected (#305). The two omitted keys are filed,
  and the evidence check now compares the object whole instead of field by field, because comparing
  named fields cannot see a key the mapper emits and the record omits. `sts2-jev-bridge --record`
  prints one `{schema, provider_call, provider_request, provider_response, decision}` object, so the
  next exchange is published from the bridge rather than transcribed from it. The default output is
  unchanged, and the runtime lane does not admit `--record` any more than it admits `--describe`,
  because that lane reads this executable's stdout as the decision itself; that refusal already
  follows from the lane's fixed admitted shape and is now pinned by a test. Refs #305.

- Consume the gateway's process-lifecycle surface through a **typed harness port**
  (`sts2-gateway-process-lifecycle-v1`, gateway pin `afb30ba9`). The harness adds three run-scoped
  management routes (capability read, one closed-action submission, identity-addressed
  reconciliation), records durable intent *before* the gateway call, and reconciles a lost response
  by operation identity instead of resubmitting it. Launch is structurally not readiness.
  Compatibility: additive-compatible; see
  [ADR 0055](docs/decisions/0055-harness-typed-process-lifecycle-port.md). Refs #101.

- **Stop a run of confident decisions cycling at a reward.** An episode played combat well for 33
  exchanges and then went round this loop until its bound expired: take the card reward (0.63), fail
  to rank three bare identifiers, skip (0.27), be offered the same reward again. The abstention bound
  cannot see it — every decision clears the gate, and every state is new because the generation
  advances, 36 distinct states across 40 exchanges. The runner now counts visits to a *situation*,
  which is the observation with `state_id` and `generation` removed, and ends the episode with
  `RepeatedSituation` once `STS2_MAX_REPEATED_SITUATIONS` (default 8) is passed. Two descriptions
  also changed, because the loop turned on what the model could read: skipping now names what is
  being declined (`skip the reward, taking none of the 3 offered`) rather than being the one tidy
  option on a screen of identifiers, and a screen that offers a set to choose from says that a card
  chosen there is kept for the rest of the run, which is framing beside the objective rather than a
  claim in an option's description. The library default for the bound is 0, the previous behaviour.
  Refs #323.

- **Stop re-asking a near-tie until a roll clears the gate.** System One is not deterministic, so
  re-asking an unchanged state re-rolls the confidence, and a run advanced when a roll happened to
  clear the gate rather than when anything was learned. One measured episode spent **140 of its 206
  provider calls** that way: the same reward screen asked four times at 0.01, 0.06, 0.17 and 0.20
  against a gate of 20. That was already acting on a low-confidence draw — it just paid for three
  refusals first and took whichever roll came up highest. An abstention may now carry the option it
  would have taken, as `candidate_action_id` and `candidate_confidence`, and the runner counts
  consecutive abstentions on one `state_id` and `generation` and dispatches that option once
  `STS2_MAX_CONSECUTIVE_REOBSERVE` (default 3) is reached. The candidate is evidence, not an
  instruction: the decision is still to observe again, an action decision may not carry one at all,
  a candidate the host no longer offers is dropped, and the settled action is validated against the
  live catalogue like any other. Output says `abstention_settled` so the record distinguishes an
  action settled under the bound from one chosen above the gate. The library default is 0, which is
  the previous unbounded behaviour, so only the runtime changes. Refs #317.
- **Let a reward say what it would offer before it is taken.** A reward is chosen on one screen and
  its contents on the next, so the first choice was made blind: a card reward was an identifier and
  nothing else until it had already been taken. In a recorded run the model committed to a card
  reward at p=0.77, found three cards it could not tell apart, skipped, and was offered the same
  reward again. A `Choice` now carries `contents`, the entries taking it would present next, and a
  reward describes as `take the reward Card reward, offering Blood Wall (upgraded) [2 energy]
  (rare): Gain 12 Block.` An entry inside `contents` has no `contents` of its own, so disclosure is
  one level deep by construction and the projection needs no depth counter to stay bounded against a
  host nesting an observation inside an observation. Additive and optional throughout: a reward that
  discloses nothing describes exactly as before, and contents listed as bare identifiers are carried
  as those identifiers rather than dropped. Refs #315.

- **Describe the options and derive the arithmetic** for the System One lane, and stop asking about
  the same play more than once. A live Linux episode recorded six options for one combat turn whose
  criteria were their own identifiers: three of them were the same Defend and two the same Strike, so
  the probability mass for playing a Defend was split three ways and the answer read as confidence
  0.19 in a turn with an obvious play. The bridge now folds strategically identical entries through
  the existing `OptionSelection`, so five catalog entries stand as three options; describes each one
  from the same observation the state carries (`play Strike [1 energy] at Nibbit (44 hit points
  left)`), so no identifier has to be resolved against the hand; and adds `DerivedExactFacts` to the
  state, which states gross incoming damage, survival, affordable cards and the weakest enemy. Both
  modules already existed, were reviewed and merged, and were reachable from nothing. A turn with one
  legal action is now taken without a provider call at all, because asking spends a call to be told
  the only thing that can happen. `ValueKind::Card` additionally admits `description`, the host's own
  card text, which the sandbox previously refused: a host that carries it can now say what a card
  does, and a host that does not is unaffected. Nothing here invents an account of the game: every
  word of a description is either a host-supplied value or a fixed label for the host's own action
  kind, and an unlabelled kind still reads as its identifier. Refs #313.

- **Let a host describe the set it offers**, and make an optional field actually optional.
  `require_exact` counts keys, so admitting a field in the allow-list alone still refused the object
  for carrying one key too many: `description` on a card was admitted and then rejected by the shape.
  `require_fields` states required and optional fields separately, and a card may now carry the
  host's own text. `state.choices` and `state.options` accept a described entry as well as the bare
  identifier every host sends today, so a reward screen can say `choose Tremble [2 energy]
  (uncommon): Apply 3 Vulnerable to ALL enemies.` instead of `select_card:123:card:22:Tremble`. The
  identifier form is unchanged and still admitted. `skip_reward` and `proceed` are labelled rather
  than left to fall back to their identifiers. This is capacity, not behaviour: the offered set is
  unmodeled upstream, which `sts2-game-core` records as a deliberate exclusion of `RewardChoicePicks`
  because "the offered set is unmodeled, so no identity or rarity is inferred", so nothing populates
  the described form until a host does. Refs #315.
- Add the **campaign episode mode** for a local provider bridge. A local bridge previously had two
  modes to name: the combat demo and the live episode, and the live episode is restricted to the
  OpenAI Astra provider. That left `typesafe-jev` with only the combat demo, which acts solely while
  the host is already in combat and never leaves a menu, so the provider could observe a campaign but
  never begin one: against a freshly launched host it polled an unchanging main-menu observation
  until its bound elapsed and was asked for nothing. `STS2_CAMPAIGN_EPISODE=true` names the third
  mode, which runs the ordinary episode runner and so reaches the host's whole action catalogue,
  `start_run` included. It is exclusive with the combat demo rather than layered, because the two
  take different runners and a vector naming both states no intent. The bridge digest check and the
  argument allow-list are unchanged and still apply to every mode. Refs #311.

- Fix the **option-selection fold key**, which folded distinct host-listed actions. The key was built
  from a fixed list of seven identity fields, so any action whose identity lived outside that list
  collapsed into a neighbour and was recorded as an intentional duplicate: two different cards aimed
  at one enemy differed only in `card_id`, and `use_potion`, `rest_option`, `select_card`,
  `confirm_selection` and `cancel_selection` carry `potion_id`, `rest_option_id` and `selection_id`,
  which the model-view vocabulary does not declare at all. The key is now built from the whole action
  the host emitted, so it is injective on whatever the host carries, declared here or not. Exactly
  one substitution remains and it is the only thing that folds anything: a `card_id` that resolves to
  a card in hand is replaced by that card's identity — name, cost, upgraded — so two copies of one
  card aimed at the same target still fold, while two different cards, two costs, an upgrade, or a
  card that does not resolve never do. Reported against the merged #295. Compatibility: fewer options
  are withheld, and no option that the host listed can now be hidden behind an unrelated one. Refs
  #302.

- Let an operator **set the System One confidence gate** per invocation. `sts2-jev-bridge` gains
  `--gate PERCENT`, an integer percentage so an argument vector carries no locale-dependent
  separator and admission can compare it exactly; absent, the bridge's own default still applies.
  The admitted argument form for `typesafe-jev` accordingly accepts either the four-element model and
  transport pair or that pair followed by `--gate PERCENT`, and nothing else. This exists because a
  gate is a measurement rather than a taste: the first two recorded live answers came back at `0.44`
  and `0.42` against a `0.55` default, so a lane left at the default would return `reobserve` on
  states like those and never act. Changing it through the recorded argument vector keeps that
  visible in a run's identity instead of hidden in a rebuild. `--describe` reports the gate the
  invocation would use. Compatibility: additive; the existing four-element form and the default gate
  are unchanged. Refs #308.

  The HTTPS exchange is performed by an operator-owned transport executable named by `--transport`,
  following the precedent `sts2-astra-bridge` set; request construction, bounds, catalog membership,
  the confidence gate and the decision shape stay inside the digest-pinned binary, and the credential
  never reaches this process. Standard input and output are serviced on their own threads and the
  transport is killed at a deadline, so neither side can deadlock on a full pipe.
  `systemone_decision` maps the answer: an in-catalog choice at or above the confidence gate becomes
  an `action` carrying the confidence as the percentage the decision contract already accepts, and
  one below it becomes `reobserve` rather than a guess. The `rationale` is composed from the returned
  distribution and labelled bridge-authored, because this provider generates no text and a fluent
  sentence presented as model reasoning would be a fabricated record.
  Compatibility: additive; one new binary, one new support module, one new document.
  `confirmed` only for the offline suite; a live call, the TLS path, decision quality, and any
  gameplay outcome are `unverified`. Refs #284, #288.
