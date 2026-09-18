# Changelog

All notable user-visible or operational changes to this project are documented here.

The project follows Semantic Versioning once versioned releases begin. Foundation work does not
claim a released harness version or runtime compatibility.

Completed entries that no longer fit the active file's preferred size budget are preserved verbatim
in [`docs/CHANGELOG-ARCHIVE.md`](docs/CHANGELOG-ARCHIVE.md).

## Unreleased

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

- Make the harness library **compile for Windows** again, and add a lane that keeps it that way.
  `exo_lifecycle/process_effect.rs` guarded one unix-only call and then used `rustix::process` —
  whose `process` module is unix-only — unconditionally for the child's identity and for killing its
  process group, so every Windows target failed with four `E0433`s and no binary in this workspace
  could be built for Windows. Reaping moves to `exo_lifecycle/process_reap.rs`, which states the
  platform difference instead of hiding it: unix signals the child's whole process group as before,
  and Windows terminates the child itself, which does not reach a descendant the transport spawned.
  That difference is recorded in `docs/COMPATIBILITY.md`; closing it means attaching the child to a
  job object at spawn, which is a change to the spawn path rather than to reaping. Two Windows-only
  warnings in `provider_session/state_store.rs` are resolved at the same time: the `Read` import
  moves into the unix-only reader that needs it, and the mode-narrowing helper now says what Windows
  does instead of leaving its parameter dead. A check-only `x86_64-pc-windows-gnu` job builds the
  workspace and the System One bridge, so a unix-only call cannot reach the library unnoticed again;
  it runs no tests on Windows and no behavioural claim about Windows follows from it. Compatibility:
  no change on Linux; Windows moves from not compiling to compiling. Refs #301.

- Add the **`sts2-jev-bridge` executable**, which asks one typed question of a System One provider
  and returns one terminal decision. It reads a bounded decision request on standard input, builds
  the request from the host-generated catalog, runs one bounded exchange, maps the answer, and prints
  exactly one decision. Every refusal is fail-closed — a nonzero exit and nothing on standard output
  — for an oversized request, an absent or malformed catalog, a transport failure or nonzero exit, an
  unreadable or oversized reply, an answer of the wrong type, and a choice outside the presented
  options. `--describe` prints the requested configuration without reading input or starting a
  process, and reports requested configuration rather than availability.
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

- Admit a **`typesafe-jev` local bridge provider kind**, fail-closed. The kind joins `ollama` and
  `openai-astra` on the legacy local-bridge lane and keeps every guard that lane applies: the
  SHA-256 digest computed from the bytes at `STS2_EXO_BRIDGE_BINARY`, the explicit combat-demo or
  live-episode requirement, and the per-kind argument allowlist. It is not promoted to live-episode
  mode, which stays Astra-only, and not to the reviewed envelope, whose route axes bind one provider
  and host by design. Its admitted argument form is exactly
  `["--model", MODEL, "--transport", PATH]` with an absolute transport path, re-parsed with the same
  parser the bridge executable uses so admission and the executable cannot disagree about what a
  valid invocation is. Argument admission moves out of `runtime_v3_settings.rs`, which was at its
  300-line preferred budget, into `runtime_v3_settings_local_bridge.rs` with the existing Ollama
  shape and its tests. The provider credential needs no code: the bridge process is spawned with a
  cleared environment and only the names in the operator's `STS2_EXO_INHERITED_ENV_JSON` pass
  through, so `TYPESAFE_API_KEY` reaches it by name and never as an argument or a record.
  Compatibility: additive; one new accepted value, no change to an existing shape, record, or digest.
  With the kind admitted and no bridge executable present, the runtime still fails closed at digest
  verification. Refs #285.

- Build a **System One provider request** from a bridge decision request.
  `context_control::build_system_one_request` turns a rendered observation and a presented action
  catalog into the body of one typed question: a `choice` whose option identifiers are exactly the
  catalog, with the request's objective and hard constraints carried in its instruction. It sits
  beside the existing provider projection and is pure — no socket, no environment, no credential —
  so every refusal happens before any of those exist. It refuses an empty, oversized, duplicated, or
  non-printable option set, a malformed model identifier, an empty state, and a state that would
  exceed a conservative byte ceiling for the published 32k-token state-and-question budget, and it
  never truncates a state to make it fit. Serialization is byte-stable, so
  `system_one_questions_digest` gives a run record the honest analogue of the reviewed envelope's
  `prompt_digest`: a question set is data, so its digest states exactly what was asked. Exactly one
  question is asked; the provider evaluates many per call in parallel, but an unconsumed question
  would spend tokens producing a number no code reads. Compatibility: additive; one new module and
  its re-exports, no change to an existing record, route, or digest. Refs #283.

- Derive the **presented option set from the state** instead of offering a provider the whole legal
  catalog. `context_control::OptionSelection` folds catalog entries that are identical under the
  admitted action vocabulary — the same kind aimed at the same target, differing only in which copy
  of a card in hand it names — into one presented option, and records every fold with the option it
  folded into, so a replay can show exactly what the provider was and was not offered. It reports
  `forced` when one action is legal and no question is needed, `single` when the presented options
  fit one question, and `two_stage` above a declared bound, where a kind is asked before an action
  within it. Presented and withheld entries partition the catalog; presented order is catalog order
  and nothing here ranks, scores, or prefers an action. A selection that would leave fewer than two
  options presents the catalog unchanged, because one option is not a question. Affordability is
  deliberately not a withholding rule: the host lists an action only when it is legal, so filtering
  on cost could only ever overrule that authority, and affordability stays in the derived-exact facts
  beside the state. Compatibility: additive; one new module and its re-exports, no change to an
  existing record, route, or digest. Refs #290.

- Compute **exactly derivable combat facts** from an admitted observation, so a provider is handed
  comparisons rather than operands. `context_control::DerivedExactFacts` states gross incoming
  damage (revealed intent damage times hits, only when every listed enemy carries an intent), a
  `fatal`/`heavy`/`survivable` label against current hit points, the hand cards current energy
  covers, the hand cards whose cost is not a fixed number, the single lowest-hit-point enemy, and
  the two counts a model would otherwise tally itself. It reads the admitted observation only, and
  a value it cannot derive exactly is omitted rather than estimated: an unrevealed intent removes
  the damage total and says so instead of counting as zero, and a tie names no weakest enemy. Two
  boundaries come from the declared model-view vocabulary rather than from the game — a card carries
  no attack value, so no lethal claim is derivable, and nothing carries block, so incoming damage is
  gross and named to say so. The projection is `derived_exact` under the fair-play taxonomy and is
  not host authority. Compatibility: additive; one new module and its re-exports, no change to an
  existing record, route, or digest. Refs #287.

- Share one **bounded HTTP/1.1 response reader across provider bridges**. The strict reader that
  refuses oversized headers, a duplicate `Content-Length`, both framings at once, a non-`chunked`
  transfer coding, an oversized or short chunk, and any trailer after the terminal chunk moves from
  the Ollama bridge's private `runtime_support` include to the shared `bin/support` tree, where a
  second bridge reaches it the same way the Astra bridge reaches its accounting support. Refusals are
  now a typed `ProviderResponseError` carrying a stable code per cause rather than an opaque string,
  and the added negative tests pin the status, terminator, declared-length, absent-framing,
  malformed-header, and non-JSON refusals that were previously only implied. The loopback-only
  `ManagementClient` stays a separate boundary and is unchanged. Compatibility: no behaviour change;
  `sts2-ollama-bridge` accepts and refuses exactly what it did before. Refs #282.

- Record the **System One provider lane and its transport** in
  [ADR 0053](docs/decisions/0053-system-one-provider-lane.md). A System One provider evaluates typed
  questions against one state and returns structured answers with probabilities and a calibrated
  confidence rather than text, so a host-generated action catalog becomes the option set of one typed
  question and the returned distribution can gate the decision. The ADR admits it as a local bridge
  kind on the legacy lane — digest pin, argument allowlist, and explicit combat gate intact, no
  live-episode promotion, no reviewed-envelope admission, whose route axes bind one provider and host
  by design — and decides that the bridge owns the request and answer contract while a pinned,
  operator-owned executable owns the HTTPS exchange, following the precedent `sts2-astra-bridge` set.
  A pure-Rust TLS client inside the bridge is recorded as the migration path with the reasons it is
  not the first step, and the rejected alternatives are stated rather than implied. Published limits,
  price, and documented model weaknesses are carried with `source-derived` labels and their sources;
  the claim that this lane plays the game is `unverified` and no run exists. Compatibility:
  documentation only; no code, dependency, or contract changes. Refs #286.

- Remove an **orphaned `context_memory` source fragment** that made the repository impossible to
  check out on Windows. `crates/harness/src/context_memory/aux.rs` was 188 lines beginning inside an
  `impl` block and ending on a dangling attribute; nothing declared it, so no build, format, lint, or
  test ever read it, and its live counterparts are `approval.rs` and `authorizer.rs`. Because `aux`
  is a reserved Win32 device name with any extension, `git clone` on Windows stopped with
  `error: invalid path` and left an incomplete tree that could not be built. Compatibility: no
  behaviour change; the file was outside the module tree. Refs #281.

- Make the one-shot Exo bridge **advertise the variants it implements**. `--describe` publishes
  `profile_support` (`map`/`management`/`expert` `unsupported`), `decision_support` (`recovery`
  `unsupported`) and the two fail-closed codes, so a caller can pre-check support rather than infer it
  from a rejection identical for every axis. The guard walks the one axis list the advertisement is
  derived from, so an enforced axis is published in the step that enforces it; the single exempt axis
  (`revision`, already published as `source_revision`) is named in code and pinned by test. Map,
  management and expert stay negative-only. Compatibility: additive.
  See [evidence](docs/evidence/exo-advertised-variant-negatives-20260918.md). Refs #141.

- Persist **logical-invocation context lifetime consumption at dispatch admission**. A continuity
  owner issues a bounded `ContextLifetimeScope` (`ascension.context-control.lifetime.v1`) over
  ordered context item ids for one agent/episode/run — optionally pinned to one branch — with
  applicability `current_invocation` or `next_n { bound }` and a wall-clock ceiling that is an
  additional bound rather than the mechanism. Applicability is consumed at exactly one site, durable
  dispatch admission, on a logical `invocation_id` whose `attempt` distinguishes transport retries:
  a preview, reload, receipt lookup or retry never consumes, extends or resurrects applicability,
  and a retry of the same logical invocation is answered with its existing manifest instead of a
  second slot. A crash *before* the durable write consumes nothing; a crash *after* it leaves the
  slot consumed and the invocation held as a possible dispatch until reconciliation, because a
  dispatch that may have happened is never silently handed back. Counters and identities persist
  through `ContextControlStore` as `ascension.context-control.lifetime-state.v1`, so a restart
  replays the same window. Manifests are append-only and their digest binds the immutable admission
  facts, so reconciliation and expiry never rewrite or delete history. Sibling agents, branches,
  episodes and runs cannot inherit a scope. Compatibility: additive-compatible; one new table
  (`context_control_lifetime`), no change to an existing table, column, digest or route; see
  [ADR 0051](docs/decisions/0051-logical-invocation-lifetime-consumption.md). Refs #111.

- Exercise **stale-generation and not-observable bootstrap refusals against the pinned real
  Gateway and MCP** in the game-information peer-contract lane. The synthetic producer behind the
  pinned peers now selects a closed `PeerNegative` (`None`, `ForeignManifest`, `StaleGeneration`,
  `NotObservable`) instead of a boolean, and two operator-only entry tests assert the runtime exits
  non-zero, delivers no data or decision to the agent, reaches the Gateway lookup-binding route,
  issues no content query, and surfaces exactly the typed lookup error (`Reobserve`,
  `MissingCapability`) for the producer's bootstrap answer. Two harness consumer corrections were
  required for the negotiated bootstrap to be reachable at all: the MCP catalog validator now
  accepts the MCP's `live_details` capability group for the bootstrap tool (no MCP group is named
  after the tool), and the runtime RPC wrapper preserves a structured bootstrap `error_response`
  tool error so its `error.code` maps to a typed lookup error instead of an opaque transport
  failure. The workflow runs its `cargo test | tee` steps under `bash -eo pipefail`, so a failing
  peer test fails its step. The lane pins the Gateway and MCP revisions that make the bootstrap
  reachable end to end: the Gateway bounds a bootstrap request's declared limits by the pinned
  schema's maxima rather than its smaller response-framing ceiling, and the MCP forwards the sealed
  bootstrap envelope verbatim instead of injecting runtime-v1 transport identity and keeps a typed
  bootstrap `error_response` on a 5xx answer instead of collapsing it to a retryable
  `gateway_unavailable`. Compatibility: internal; no schema, route or durable record changes.
  Refs #127, #276.

- Serve the **provider-session effective-limits record** for the Console capability sidecar.
  `GET /v1/workflow-runs/{run_id}/provider-session-effective-limits` (`workflow:read`) returns the
  producer's `ascension.harness.effective-limits.v1` record built by
  `NativeCapabilities::effective_limit_record` from the descriptor the served process admits
  provider sessions against: metadata only, validated before it is returned, and fenced to the
  run's current context-owner association (`provider_session_capabilities_mismatch` when the
  boundary names another adapter/model revision; `provider_session_capabilities_unavailable` when
  no descriptor is served, never a fixture). The served workflow composition holds no memory
  corpus, so `GET /v1/workflow-runs/{run_id}/context-memory-effective-limits` refuses with the
  typed `context_memory_record_unavailable`. `docs/COMPATIBILITY.md` is split: the context-owner
  rows move to `docs/COMPATIBILITY_CONTEXT_OWNER.md`, where the control-command route regains its
  own heading. Compatibility: additive-compatible; see
  [ADR 0052](docs/decisions/0052-provider-session-effective-limits-route.md).
  Refs AI-Ascension/ascension-context-console#18.

- Consume the shared `game-information-live-observation-bootstrap-v1` conformance case and its
  seven invalid fixtures (copied byte-identically from sts2-protocol, `SHA256SUMS` extended) and
  drive the `error-native-unavailable.json` golden through the MCP bootstrap boundary: a
  `not_observable` error is the typed missing-capability result, installs no snapshot, retains no
  producer text and delivers nothing to the agent. Safety correction in bootstrap snapshot
  selection: a visible entity carrying a foreign content manifest is now rejected instead of
  skipped, and the response selector must echo the request selector exactly. No schema, digest,
  route or durable record changes. Refs #127.

- Submit **context-owner control commands over management HTTP**.
  `POST /v1/workflow-runs/{run_id}/context-control-commands` forwards one `pause`/`commit`/`resume`
  `ContextControlCommand` to the authoritative context owner for the run's current binding under
  scoped `workflow:control` and returns the owner's `ascension.context-control.owner-receipt.v2`.
  The harness mints no authority: an exact duplicate returns the recorded receipt without a second
  effect, and a stale control version, boundary or revision fence is refused with a typed conflict
  before the owner is called. A served profile may also set `STS2_WORKFLOW_TOKEN_<PROFILE>_READ`
  to mint a `workflow:read`-only companion token for the same subject, so a metadata-only caller is
  refused with `missing_scope` on content writes, adoption and control. Compatibility:
  additive-compatible; see [ADR 0048](docs/decisions/0048-context-owner-control-commands.md).
  Refs AI-Ascension/ascension-context-console#18.

- Execute independent read-only analyses of an admitted dynamic plan under an **owner-enforced
  in-flight cap** from `WorkflowLimits::max_parallel_analyses`, joined by node identity:
  `execute_plan_bounded` records each node `Settled`, `Failed` or `Unknown` in a `JoinedResult`
  whose `join_digest` is identical for every completion order, never dispatches a node whose
  declared input did not settle, and records an unwinding branch as `BranchLost` rather than
  stalling. `ParallelCap::SERIAL` keeps cap=1 compatible and `execute_plan` is unchanged; this is
  additive, and budget reservation, cancel/restart and browser branch state remain open. See [ADR 0049](docs/decisions/0049-bounded-parallel-analysis-join.md). Refs #98.

- Deny **forbidden Exo tools by name at dispatch** in the owned restricted extension and re-record
  the real pinned-Exo process oracle. The model tool catalog is empty; the extension now also seals
  the actual `HarnessToolRegistry` handed to each model round, so a pre-populated registry, any
  later `register`, and any `executePending` — `shell`, `install_agent_tool`,
  `uninstall_agent_tool`, `manage_tool`, `inspect_tools`, `install_skill`, `remember`,
  lookup-profile tools, and any case/namespace variant — throws the typed `sts2_forbidden_tool`
  error before a handler can exist and records the denial counts in a new `sts2.exo-tool-guard-v1`
  event. The executor requires that event and maps a non-zero count to receipt
  `error_code: exo_forbidden_tool` with no decision; the bridge fails closed as
  `exo_bridge_executor_failed` after exactly one model egress. New process-oracle cases
  `forbidden_tool_by_name_*` (one per name/alias, driven by a synthetic model that calls the tool)
  and `request_tools_are_empty` exercise both the bridge and the executor boundary against the real
  pinned Exo with a synthetic loopback model (no provider, no game); the shipped extension digest,
  the recorded oracle bytes, and `protocol-artifact/exo-bridge-v1/{manifest.json,SHA256SUMS}` are
  re-recorded together so `crates/harness/tests/support/exo_contract_process_evidence.rs` stays
  fail-closed. Also documents `STS2_EXO_PRIVATE_STATE_ROOT`, the truthful capability list, and
  source-freeze/re-admission in the new `docs/exo-compatibility.md` (the Exo sections of
  `docs/COMPATIBILITY.md` moved there unchanged to stay within the file budget), and corrects stale
  `experiments/exo-agent/README.md` lines that predated runtime-v3 admission (#205/#223/#226).
  Compatibility: `safety-correction` to an unreleased candidate — the empty registry is now
  enforced in dispatch rather than inherited from upstream; no wire field, route, published schema,
  or durable record changes. Refs #140.

- Consume the gateway's negotiated **repeated-episode lease profile** so a harness run can
  complete two episodes against one gateway deployment. The gateway permanently revokes its local
  lease context on a successful `release`, so a second episode could never be admitted
  (AI-Ascension/sts2-gateway#67). A run that opts in with `STS2_EPISODE_PROFILE=true` sends
  `x-sts2-episode-profile: repeated-episode-lease-v1` on the release that completes an episode and
  requires the gateway's exact witness back (`profile`, `capability`, `schema_digest`,
  `released_epoch`); a missing or mismatched witness fails the run instead of silently degrading to
  the single-episode default. The profile is armed **only** for a completed episode: the episode
  runner marks completion solely on a successful terminal outcome, so every failure, cleanup, and
  restart path keeps the gateway's fail-closed permanent revocation. The witness is required only
  when this build negotiated it, so a run that does not opt in sends no header and its release body
  stays byte-identical. Compatibility: opt-in and additive — no field, route, durable record, or
  published schema changes, and the profile is off by default. See
  [gateway ADR 0033](https://github.com/AI-Ascension/sts2-gateway/blob/main/docs/decisions/0033-repeated-episode-lease-profile.md).
  Refs AI-Ascension/sts2-gateway#67.

- Stop reporting a **failed live command as settled**. A command that faults before executing
  anything (`live_execution_failed`) is still `CommandOutcome::Applied` — the command was processed
  and its response vocabulary is unchanged — but its event is no longer classified `settled`. The
  run fails, the cursor stays on the same node, and no provider call is consumed, so a `settled`
  classification presented a failure as forward progress to consumers that treat a settled step as
  completed. Classification is now derived from the outcome *and* the reason code in one shared
  place (`CommandOutcome::classification`) for both the memory and SQLite event writers, replacing
  two copies of the mapping. A new `CommandOutcome` variant was rejected because it would extend the
  closed `ascension.management/v1` outcome set without the version negotiation a published consumer
  change requires; the event classification enum already publishes `rejected`
  (`ascension.workflow-event/v1`). Compatibility: `safety-correction` to an unreleased candidate —
  one failure path now emits `rejected` instead of `settled`, and no field, route, durable record,
  or published schema changes. See
  [ADR 0047](docs/decisions/0047-failed-command-event-classification.md). Refs #260.

- Wire the per-invocation **context membership boundary** into the production render path so a
  policy actually changes published application bytes. `ContextMembershipSelector` is the
  owner-configured half (disposition, overrides, pin inheritance, wider scope, model view) and
  `bind` mints the versioned `ascension.context-control.membership.v1` policy for one invocation of
  one draft revision, so a default or override cannot silently carry another invocation's identity.
  The live managed dispatch seam and the composed owner render seam both resolve and gate the
  effective set before any provider bytes exist, project the model-visible subset onto a cloned
  draft, narrow pins to model-visible ids, and then delegate to the renderer so the selected owner
  limits still compose with (narrow) the membership bound instead of replacing it. An invocation
  without a selector renders today's exact bytes. `MembershipContinuity` is derived from the
  binding's `provider_session_continuity`. Effective absence is refused for **every** continuity
  until the render path can actually omit the observation from the composed provider request; the
  stateless case was refused too after #254 proved it was admitted while the observation still
  shipped in the served bytes. Refusals name the precise pre-dispatch gate
  (`context_membership_*`) rather than a generic provider failure.
  Compatibility: additive — no existing field, route, durable record, or published schema changes.
  Owner continuations gain optional `membership` configuration; absence preserves current behaviour.
  See [ADR 0046](docs/decisions/0046-invocation-context-membership.md). Refs #106.

- Enforce a versioned, per-invocation **context membership policy** so one invocation can include,
  exclude, or inherit collected context independently of the persisted draft. `resolve_membership`
  records a typed reason per reference and binds the decision with a policy digest; scope is carried
  by the item kind, so an invocation-scoped item is refused as a sibling-scope leak unless an
  explicit wider-scope authorization names both the calling agent and that item.
  `prevalidate_and_bind` fails before dispatch on revoked, expired, or digest-mismatched items, on a
  protected owner prerequisite a policy tried to exclude, and on the mandatory-plus-pin and effective
  item bounds. A model-view policy may express an omitted observation while the item is retained as a
  mandatory prerequisite for owner legality, but that effective absence is refused for every
  continuity: an opaque persistent adapter cannot claim a selector erased provider history, and the
  render path has no omission wireform, so an admitted stateless invocation would report the
  observation hidden while still publishing it. Unpin and exclusion change only the next prepared
  input, and `EffectiveMembership::revalidate` refuses anything that moved since preparation.
  Compatibility: additive — no existing field, route, durable record, or published schema changes;
  the new `ascension.context-control.membership.v1` policy is in-process with no published consumer.
  See [ADR 0046](docs/decisions/0046-invocation-context-membership.md). Refs #106.

- Refuse recorded-run export explicitly on non-Unix platforms, where its descriptor-relative
  no-follow snapshot reader is unavailable, instead of preventing the whole harness from compiling.
  The Unix snapshot checks remain intact. This does not certify Windows runtime behavior.

- Inspect the actual Exo launch configuration using the same loader as the bridge. Bind the
  configured extension, prompt, tool catalog, model route and configuration to the operator pins,
  require the package locator to match the configured executor, and use the shared artifact bounds.
  Envelope arguments now require `--run`, an absolute configuration path and its digest.
  Compatibility: breaking operator configuration, unchanged wire schemas. Unverified lifecycle
  capabilities remain an independent admission gate; no provider/native certification is claimed.
  See [ADR 0032](docs/decisions/0032-inspected-admission-identity.md). Refs #139.

- Bind the **package axis** of the runtime Exo admission identity to an inspected artifact. The
  reviewed `envelope` mode now requires `STS2_EXO_PACKAGE_PATH`, reads the exact bytes it locates
  through the existing bounded inspection read, and hashes them into `package_digest`, so a swapped
  package is refused as `IdentityMismatch("package_digest")` before the capability gate instead of
  incidentally as `UnboundIdentity("package_digest")`. The inspected digest is computed from the
  located bytes and the operator's `STS2_EXO_PACKAGE_DIGEST` is never substituted for the
  observation, so the pin stays independent. The remaining axes still have no inspected artifact, so
  the envelope still refuses every deployment today, now naming `extension_digest` as the first
  unbound axis. Compatibility: `breaking` for operator configuration — the locator is required and a
  deployment without it fails closed with `STS2_EXO_PACKAGE_PATH is required`; no wire field,
  schema, contract version or durable record changes, and `legacy` behaviour is unchanged. See
  [ADR 0032](docs/decisions/0032-inspected-admission-identity.md). Refs #139.

- Renumber eight harness decision records whose numbers were each held by two different records, so
  every `ADR NNNN` label and `NNNN-*.md` link denotes exactly one decision. The moved records and
  every in-repo citation site were updated in the same change; no decision content changed. See
  [docs/decisions/README.md](docs/decisions/README.md) for the old-to-new mapping. Refs #203.

- Cross-check the **inspected Exo deployment identity** against the operator pin at the runtime
  admission boundary, so a swapped package, extension or bridge artifact fails closed instead of
  being admitted on the operator's declaration alone. `ExoAdmissionPlan::inspected` derives the
  advertised identity from the SHA-256 of the inspected artifact bytes, `preflight` compares that
  identity before the capability gate (a swapped artifact is now `IdentityMismatch(axis)` rather than
  a masked `RequiredCapability`), and a pinned axis the inspection did not bind is refused as
  `UnboundIdentity(axis)`. The runtime seam inspects the bridge executable's bytes and deliberately
  does not copy the operator's environment into the inspected identity, so the remaining pinned axes
  stay unbound and the reviewed `envelope` mode still refuses before any model or game effect.
  Compatibility: `breaking` for the refusal vocabulary and operator messages only — no wire field,
  schema, contract version or durable record changes, and `legacy` behaviour is unchanged. See
  [ADR 0032](docs/decisions/0032-inspected-admission-identity.md). Refs #139.

- Invoke the pinned-Exo **capability preflight at the runtime transport seam** so a missing,
  malformed, unknown or unverified deployment fails closed before a model or game effect.
  `STS2_EXO_ADMISSION` selects the mode: `envelope` (the default when unset) assembles the
  operator-trusted identity, refuses the run while settings are still being assembled when a
  capability, digest, revision, route or schema is not admitted, and then admits a correlated turn
  through `ExoAdmittedTransport`; `legacy` is the explicit acknowledgement of an un-admitted
  raw-wire bridge and preserves the previous behaviour. Compatibility: `breaking` for operator
  configuration only — no wire field, schema, contract version or durable record changes, the
  reviewed capability axes are not promoted on the bridge's behalf (so `envelope` currently refuses
  the current deployment; the inspected-identity entry above strengthens the reason), and the
  raw-wire development bridges need `STS2_EXO_ADMISSION=legacy`. Per-turn envelope admission for a
  multi-turn episode remains open. See
  [ADR 0031](docs/decisions/0031-runtime-exo-admission-gate.md). Refs #139.

- Keep a **cancel** pending as `NeedsOperator` while a live operation's settlement is still unknown,
  instead of stopping the episode and marking the run cancelled. `CommandKind::Cancel` reconciles
  first, and when reconciliation reports `ErrorClass::Unresolved` it returns
  `CommandOutcome::Pending` with reason `live_operation_unknown`, leaving the pending operation
  identity, the `NotStarted` cleanup state and the live session untouched so that an operator can
  still settle it. Compatibility: the cancel command, its revision guard and its response schema are
  unchanged, and a cancel at a settled revision behaves exactly as before. Refs #94.

- Record the **real pinned-Exo one-shot executor process oracle** against the shipped extension
  bytes and bind it mechanically: `docs/evidence/exo-executor-process-oracle-20260915.{md,json}`
  capture a reproduced run of the real pinned Exo runtime through `sts2-exo-bridge` →
  `sts2-exo-executor` with an original synthetic loopback model (27/27 cases, four correlated
  terminal decisions, retry containment to one egress, no provider, no game), the artifact manifest
  gains a `process_evidence` record, and workspace tests now fail closed when the extension module,
  the oracle source, or the pin inventory drifts from that record. Compatibility: additive;
  no schema, wire field, contract version, or runtime behaviour changes. See
  [the record](docs/evidence/exo-executor-process-oracle-20260915.md). Refs #139.

- Tolerate a **transiently held** owner lease instead of reporting it as busy. `Lease::acquire` now
  retries the non-blocking lock attempt for a bounded interval, because an `flock` belongs to the
  open file description: a descriptor this process has already closed can still be held by a spawned
  child until it reaches `execve`, which can briefly outlive the owner that closed it. Exhausting the
  attempts still returns `Busy`, and the lock primitive, its exclusivity and its release on process
  death are unchanged. This removes the intermittent `restart: Busy` failure of the `exo_lifecycle`
  tests under parallel execution. Compatibility: no file-format, schema, range or bound change; a
  genuinely busy lease is now reported after about 160 ms (32 attempts, 5 ms apart) instead of
  immediately. See
  [ADR 0029](docs/decisions/0029-owner-lease-transient-busy-retry.md). Refs #188.

- Compose the authoritative context owner's **current** binding with the catalog descriptor that
  admits it, and publish the resulting effective limits as the versioned read-only projection
  `ascension.harness.context-owner-effective-limits-view.v1` over
  `GET /v1/workflow-runs/{run_id}/context-owner-effective-limits`. Live admission and the
  observable surface now share one fail-closed seam, so the limits a consumer can read are the
  limits the run was admitted under — not the portable schema maxima or the harness maxima. A
  foreign owner, a missing or disabled descriptor, a binding that is not the published descriptor
  identity, a grant escalation, an oversized descriptor and a stale descriptor/catalog digest are
  each refused with a precise error; an unattached owner stays explicitly unavailable.
  Compatibility: additive read-only route; no bound, schema, digest or default changes, and the
  admission checks keep their existing error codes. See
  [ADR 0030](docs/decisions/0030-context-owner-effective-limits-composition.md). Refs #95.

- Enforce the **selected** context-control limits that a binding advertises instead of only the
  harness maxima: `ContextRenderer::enabled_at_with_limits` refuses a draft that exceeds the
  advertised `max_items`, `max_notes`, `max_objective_bytes` or `max_context_bytes` with a precise
  error naming the limit, before any inference or retention. Compatibility: additive; `enabled`,
  `enabled_at` and `legacy` keep their signatures and behaviour, and no bound changes. See
  [ADR 0028](docs/decisions/0028-selected-context-control-limit-enforcement.md). Refs #95.

- Add bounded migration records for saved provider-session policies that are portable-schema valid
  but above the selected profile's executable ceiling: the exact saved bytes and violated limits are
  retained, and adoption requires explicit approval plus a caller-supplied target that is already
  within the executable ceilings — no value is ever silently clamped. Compatibility: additive,
  library-only; no policy field, schema, range or bound changes. See
  [ADR 0027](docs/decisions/0027-provider-session-policy-migration.md). Refs #95.

- Classify saved provider-session policies precisely against the **selected** adapter profile:
  `ProviderSessionPolicy::admit_for_profile` checks portable schema validity separately from the
  profile's executable ceiling and returns either a schema failure or a precise capability reason,
  never a generic invalid-policy error and never a silent clamp. Compatibility: additive; no policy
  field, schema, range or bound changes. See
  [ADR 0026](docs/decisions/0026-provider-session-saved-policy-admission.md). Refs #95.

- Expose the authoritative context owner's **current** association for one workflow run as the
  versioned read-only projection `ascension.harness.context-owner-association-view.v1` over
  `GET /v1/workflow-runs/{run_id}/context-owner-association`. The projected grants and epochs are
  owner assertions, not harness-issued authority; a binding for another run fails closed and an
  unattached owner stays explicitly unavailable. Compatibility: additive read-only route; the
  existing `ContextAssociation` route is unchanged. See
  [ADR 0025](docs/decisions/0025-context-owner-current-association.md). Refs #100.

- Add read-only recovery of already-issued context-control receipts. A caller whose delegated
  `pause`/`commit`/`resume` reply was lost can now look up the owner's recorded receipt by replaying
  the exact command instead of re-issuing it, gated on the binding advertising `receipt_recovery`.
  A recovered receipt must satisfy exact owner/invocation/binding/command identity before it is
  returned. Compatibility: additive, read-only, failing default port method; no schema, record,
  digest or resource bound changes. See
  [ADR 0024](docs/decisions/0024-context-control-receipt-recovery.md). Refs #100.

- Expose the recorded context-owner binding for one workflow invocation over the authenticated
  management HTTP surface as a separately versioned, read-only projection. Same-subject scoped
  `workflow:read` is required; an unrecorded invocation, another subject, a missing scope and
  disabled retention are reported as distinct errors. Compatibility: additive read-only endpoint;
  no existing route, record, schema or resource bound changes. Does not establish current owner
  authority or receipt recovery. See [ADR 0023](docs/decisions/0023-recorded-context-binding-http-projection.md).
  Refs #100.

- Add an opt-in Exo duplex lookup bridge and bounded native TypeScript tool registration,
  connecting the lookup agent API to the isolated pinned executor. See
  [ADR 0022](docs/decisions/0022-exo-lookup-duplex-bridge.md). Refs #127.

- Add scoped game-information v1 lookup consumption through the existing MCP port, a bounded
  typed agent tool loop, complete-source validation before projection, separate source/view
  identities and encrypted pinned replay archives. The opt-in mixed catalog preserves legacy
  profiles. Synthetic tool-loop and SQLite restart evidence do not claim native provider or
  exact-host execution; the old native Exo adapter remains terminal-decision-only.
  See [ADR 0039](docs/decisions/0039-game-information-consumer.md). Refs #127.

- Add the opt-in immutable benchmark manifest library: bounded strict v1 parsing, separate
  gameplay/experiment/occurrence identities, exact mismatch reasons and keyed public references.
  Existing seed receipts can be associated with an immutable planned trial as `seed_receipt_bound`
  only when the declared protocol version and schema digest also match; this is offline consistency,
  not native reproducibility or hidden RNG verification. No runtime or legacy-record behavior
  changes. See [ADR 0021](docs/decisions/0021-benchmark-manifest-foundation.md). Refs #121.

- Add opt-in bounded SQLite history for context-owner bindings, committed atomically with
  command results and read by original invocation with current scoped, same-subject permission.
  Historical grants and epochs never authorize current control or claim a restored owner.
  Public JSON schemas and current-cursor association stay unchanged; Rust `CommandApplication`
  constructors must supply the new optional `context_binding` field. See
  [ADR 0040](docs/decisions/0040-recorded-context-binding-history.md). This library-only slice
  does not implement HTTP history, owner receipt recovery or complete #100 acceptance.

- Reconcile the Exo bridge contract inventory for #139. Add executable conformance vectors for
  unavailable `map`/`expert` profiles, absent context continuity, incompatible capability
  schema/contract versions, and malformed descriptor shapes, plus a manifest pin-location drift
  guard and an explicit upstream dependency/prerequisite record. Compatibility: contract-vector and
  documentation additions only; no schema or wire field changed. Real provider/native acceptance
  remains gated by #149.

- Complete the #139 Exo pin inventory: list every revision-bearing bridge, contract, documentation
  and artifact source in the manifest and enforce the full set in the drift guard. Refresh the
  manifest checksum. Compatibility: inventory-only; no schema or wire change.

- Require game-information v1 responses with `unavailable` or `not_observable` coverage to carry an
  unknown total (`total_count_known: false`, `total_count: null`). The harness consumer previously
  rejected only non-empty pages, accepting a known or fabricated total for coverage extremes and
  thereby violating the "never convert unknown into zero or empty" rule. Compatibility: validation
  tightening only; no schema, wire field, or contract version changed.

- Bind Exo lifecycle polling and result reads to the admitted execution-store incarnation,
  and validate exact decision/reservation metadata before completion or uncertainty writes.
  Validate authenticated lifecycle-to-broker references and phase relationships before
  restart claim publication, retaining held recovery and historical completed entries.
