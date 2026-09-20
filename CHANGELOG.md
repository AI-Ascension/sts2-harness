# Changelog

All notable user-visible or operational changes to this project are documented here.

The project follows Semantic Versioning once versioned releases begin. Foundation work does not
claim a released harness version or runtime compatibility.

Completed entries that no longer fit the active file's preferred size budget are preserved verbatim
in [`docs/CHANGELOG-ARCHIVE.md`](docs/CHANGELOG-ARCHIVE.md).

## Unreleased

- Retain and query the **semantic run history** the game-mod now states. A new
  `sts2_harness::semantic_history` surface admits the producer's semantic event batches and keeps them
  as a durable, queryable history: a disclosed gap keeps its sequence number and carries no gameplay
  detail, a causal parent is stored only when the producer stated one, sequence order is monotonic and
  contiguous inside one run, branch, episode and epoch, and a read that names another scope is refused
  rather than answered from this history. Queries narrow by kind, origin, coverage, subject identity
  and sequence range, are bounded, and resume from a continuation; a causal traversal walks stated
  parents backwards and refuses rather than truncates when it would exceed its bounds or revisit an
  event. Re-appending an identical batch is a no-op, reusing an append identity with a different
  payload is refused, and a fork inherits its ancestor by lineage and appends only what is new.
  Retention over that history is explicit and reference-aware: a policy an operator must disable
  deliberately can select an observed span for pruning, a record another surviving record still
  names as its stated cause is kept anyway (and that protection closes over the ancestor chain), and
  every pruned span keeps its sequence number as a declared gap carrying the retention label, so
  nothing a policy removed can read as a measured zero. A prune plan is computed against the exact
  bytes it previews and is refused when that history has moved, and a later append still declares the
  span retention disclosed.
  Compatibility: additive; no wire field, schema or durable record changes, and no native event
  capture is claimed. See [ADR 0057](docs/decisions/0057-retained-semantic-history.md). Refs #128.

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

- Serve an **immutable inference-profile catalog** and admit typed profile bindings. `GET
  /v1/inference-profiles` (`workflow:read`) returns a sealed, credential-free
  `ascension.inference-profiles/v1` catalog of `ascension.inference-profile/v1` descriptors (adapter,
  requested/resolved model, prompt/configuration revision, supported settings, operation allow-list,
  context compatibility, continuity, effective budgets, select/edit grants, availability state). Live
  admission resolves every `decision_profile_ref`/`planner_profile_ref` to an exact id/version/digest
  revision before any reservation or provider exists, and persists the requested/resolved provenance
  on the run's target admission. Unknown id, digest mismatch, revocation, and unsupported/stale/disabled
  model or settings refuse before inference; a catalog refresh re-reads the owner and admits nothing on
  its own. Reading the catalog confers no edit path, and adopting a newer revision changes a new
  definition only — an admitted run keeps the revision it resolved (see
  [ADR 0054](docs/decisions/0054-inference-profile-revision-adoption-scope.md)). Compatibility: additive;
  a versioned closed schema (`contracts/inference-profile/catalog.schema.json`) and one new route.
  Evidence is synthetic/component only. Refs #104.

- Remove an **orphaned `context_memory` source fragment** that made the repository impossible to
  check out on Windows. `crates/harness/src/context_memory/aux.rs` was 188 lines beginning inside an
  `impl` block and ending on a dangling attribute; nothing declared it, so no build, format, lint, or
  test ever read it, and its live counterparts are `approval.rs` and `authorizer.rs`. Because `aux`
  is a reserved Win32 device name with any extension, `git clone` on Windows stopped with
  `error: invalid path` and left an incomplete tree that could not be built. Compatibility: no
  behaviour change; the file was outside the module tree. Refs #281.

- Bind the Exo **evidence records to the revision that actually contains them**. Both oracle reports
  derive `harness_revision` from `git rev-parse HEAD` while every other digest is computed from the
  worktree, so a run on an uncommitted tree emitted a record naming a revision without the evidence
  it binds — the 2026-09-17 record named `deb5df6d`, where the extension, the oracle and both
  support modules differ or are absent. `support::assert_sources_are_committed` now fails the run
  when a recorded source differs from `HEAD` or is untracked, both records are re-recorded at the
  revision carrying their bytes, and the coupled manifest/`SHA256SUMS` digests are re-pinned. Refs #140.

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
