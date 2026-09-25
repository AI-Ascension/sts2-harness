# Changelog archive

This file preserves the completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog exceeded its preferred size budget.
Entries are unchanged from the revision that introduced them; this archive is a verbatim record, not
a supported release or a second normative changelog. The closed 2026-09-10 bounded-campaign wave
that preceded the entries below is preserved in
[`CHANGELOG-ARCHIVE-2026-09-10.md`](CHANGELOG-ARCHIVE-2026-09-10.md), the entries archived on
2026-09-23 are preserved in
[`CHANGELOG-ARCHIVE-2026-09-23.md`](CHANGELOG-ARCHIVE-2026-09-23.md), the entries archived on
2026-09-24 are preserved in
[`CHANGELOG-ARCHIVE-2026-09-24.md`](CHANGELOG-ARCHIVE-2026-09-24.md), and the entries archived on
2026-09-25 are preserved in
[`CHANGELOG-ARCHIVE-2026-09-25.md`](CHANGELOG-ARCHIVE-2026-09-25.md).

### Archived from CHANGELOG.md

- Persist validated Runtime-v3 action-wait settlement against its original durable operation
  before admitting another model decision. Previously the host could settle the action while
  the durable store retained `unknown`, causing the next decision to fail with a misleading
  provider-malformed error. Unresolved waits and invalid witnesses retain durable uncertainty.
  Synthetic runtime tests cover decision admission, repeated waits, and database reopening;
  native campaign validation remains a separate gate.

- Tighten optional seeded-receipt replay admission before `EpisodeRunner` construction. A receipt
  preamble now requires exact current seed configuration and original operation/fence/context
  equivalence, a fresh canonical-seed run-start witness, and a closed MCP wrapper/result chain.
  This is deterministic source/component validation only; it does not execute replay or invoke a
  provider, MCP server, gateway, or host.

- Digest all serialized Runtime-v3 telemetry lineage identities with domain-separated SHA-256
  values while retaining raw trace lineage only for private OTLP topology derivation. Exporter
  tests cover raw prompt, model-output, credential, path, and proprietary-text sentinels through
  the full serialized OTLP envelope. Backend queries must use the deterministic digest while raw
  mappings remain access-controlled local run evidence; collector/backend and live evidence remain
  unverified.

- Add the bounded `coop-native-v1` cohort coordinator and canonical-peer attribution safety
  correction. A returned observation can be attributed only when its sole local peer equals the
  originally scheduled canonical actor and its instance/session/lease/epoch fence matches the
  original operation. Mismatches retain pending or unknown operations for same-operation
  reconciliation. The frozen artifact, wire profile, digest, and producer goldens are unchanged;
  route credentials remain outside harness records. This is source/component evidence only, not
  native multiplayer transport, settlement, or release compatibility.

- Reject incomplete exact checkpoint manifests, inconsistent dependency sizes and payload
 identities, and source/destination profile mismatches before session admission. Bound reads of
 persisted exact artifacts to 16 MiB. These checks establish component integrity, not live restore
 certification; see ADR 0016.

- Add a pinned runtime-peer CI lane. It builds the candidate harness against immutable gateway and
  MCP executable peers, uses a bounded synthetic mod HTTP endpoint only as downstream, and runs
  positive plus foreign-identity and malformed-envelope rejection cases. Startup and cancellation
  cleanup regressions run in the same lane. This is source-derived synthetic process composition
  evidence, not game-host, provider, or release qualification.

- Require a closed `ExoRestrictedProfile` in the trusted Exo configuration for issue #140.
  Admission now fails closed before inference on any non-empty/unreviewed model tool, duplicate or
  invalid tool names, unsafe or overlapping private state/cache/temp roots, unbounded
  quota/retention, or permissions other than `0o700`, with canonical catalog and profile digests.
  The reviewed model tool allowlist is intentionally empty; TypeScript dispatch, OS containment, and
  native private-state enforcement remain follow-up work.

- Define the harness-owned `sts2-exo-bridge-v1` contract and freeze the candidate Exo source
  manifest. The closed capability/preflight and request/turn envelopes enforce independent
  identity, bounds, UTF-8/framing, correlation, terminal-decision, cancellation, and EOF rules;
  deterministic fixtures cover rejection vectors. The nine-commit upstream review found no native
  machine executor hook, so package, model, extension, native connectivity, and gameplay evidence
  remain `unverified`; see ADR 0017.

- Bind the authoritative context owner at the live invocation the runtime actually executes.
  Live admission now performs a fail-closed owner/catalog/support check only; each context-bound
  node is bound when the runtime cursor reaches it, using the runtime-allocated `node_execution_id`
  and validating the owner response against the persisted run cursor. Compatibility: no serialized
  snapshot/event schema change. This is a source-level change to the management API: `CommandContext`
  gains a required actor-scoped owner field (external struct literals must supply it), the
  `WorkflowExecutionPort::attach_context_owner` hook added by #167 is removed (implementations
  overriding it must drop the override), and `CommandContext` now uses a manual `Debug` that omits
  the owner. Owner `bind` denials/escalations now surface at the first context-node dispatch instead
  of submission, while missing/denied/unavailable/ambiguous catalogs still fail closed before any
  target or execution effect. Refs #100.

- Publish the machine-readable `ascension.harness.effective-limits.v1` classification record so every
  advertised context-memory and provider-session value is classified as schema-valid versus
  executable for the selected profile, with machine-readable unavailable reasons. Add a
  producer/consumer pin and digest conformance matrix (`contracts/effective-limits-pins.json`) that
  recomputes producer digests and fails closed on drift, tampering, a stale adoption label, an
  unrecorded surface, or a consumer that still validates a `v1` capability schema. Both recorded
  consumers (Context Console, Studio) remain `pending`, so neither can present a value the runtime
  rejects. Deterministic offline tests only; consumer adoption, native, and provider evidence remain
  `unverified`.

- Add an opt-in authenticated durable memory-policy owner: retain exact saved encodings, require
  target-bound approval for atomic adoption, and load the adopted policy for actual local memory
  selection. Restart requires explicit revalidation; stale grants/profiles/revisions fail closed.
  The separate encrypted bounded store preserves history and idempotent receipts. This is the
  memory-only component slice of #95, not session migration, browser/production wiring or #119.

- Add producer-generated context-control catalog fixtures for descriptor minima, restricted
  values, global ceilings, disabled metadata and digest consistency. A catalog sealing helper
  reuses the existing v1 encoding. No wire fields, limits or selected-limit execution behavior
  change; consumer adoption and rendering/journal enforcement remain separate. Refs #95.

- Align the reviewed Console and Studio v3 effective-limit consumer pins, require exact
  repository-owned CI references, and verify candidate producer-library bytes against both
  unchanged consumer fixtures before admission tests. Validate actual static YAML checkout
  steps with pinned `yaml-rust2` 0.13.0; shell text, ambiguous mappings and conditional/inert
  steps cannot establish a consumer pin. Preserve the separate Studio workflow-owner
  regression; effective-limit evidence remains synthetic only.

- Add the owned `sts2-exo-bridge` single-turn process entrypoint and an isolated, exact-pinned
  real Exo embedding package. Strict standard/fresh requests retain their complete catalog and
  constraints; correlated output is independently parsed with no fallback. The tool-free extension
  forwards at most one model request and records denied upstream SDK retry attempts. Add
  `ExoAdmittedTransport` for full-preflight envelope handoff with explicit host identities.
  Compatibility: additive opt-in source/process path; full runtime admission, map/expert/recovery,
  durable lifecycle, actual provider/game execution and replay remain separately gated. Refs #141.

- Add opt-in Exo owner persistence with a separate encrypted broker journal, lifetime owner lock,
  authenticated send/result fences, conservative restart handling, and explicit v1 cutover.
  Existing execution-store reservations and result bytes retain their ownership. See
  [ADR 0020](decisions/0020-exo-owner-journal-and-single-use-send.md).
  Focused source/recording-fixture and local process coverage passed 37 test functions at
  `29d256c`; this is not native Exo or full-runtime acceptance. Cancellation, native
  reconciliation, qualified accounting, containment and episode admission remain gated. Refs #142.
- Bind Exo lifecycle polling and result reads to the admitted execution-store incarnation,
  and validate exact decision/reservation metadata before completion or uncertainty writes.
  Validate authenticated lifecycle-to-broker references and phase relationships before
  restart claim publication, retaining held recovery and historical completed entries.
- Require game-information v1 responses with `unavailable` or `not_observable` coverage to carry an
  unknown total (`total_count_known: false`, `total_count: null`). The harness consumer previously
  rejected only non-empty pages, accepting a known or fabricated total for coverage extremes and
  thereby violating the "never convert unknown into zero or empty" rule. Compatibility: validation
  tightening only; no schema, wire field, or contract version changed.
- Complete the #139 Exo pin inventory: list every revision-bearing bridge, contract, documentation
  and artifact source in the manifest and enforce the full set in the drift guard. Refresh the
  manifest checksum. Compatibility: inventory-only; no schema or wire change.
- Reconcile the Exo bridge contract inventory for #139. Add executable conformance vectors for
  unavailable `map`/`expert` profiles, absent context continuity, incompatible capability
  schema/contract versions, and malformed descriptor shapes, plus a manifest pin-location drift
  guard and an explicit upstream dependency/prerequisite record. Compatibility: contract-vector and
  documentation additions only; no schema or wire field changed. Real provider/native acceptance
  remains gated by #149.
- Add opt-in bounded SQLite history for context-owner bindings, committed atomically with
  command results and read by original invocation with current scoped, same-subject permission.
  Historical grants and epochs never authorize current control or claim a restored owner.
  Public JSON schemas and current-cursor association stay unchanged; Rust `CommandApplication`
  constructors must supply the new optional `context_binding` field. See
  [ADR 0040](decisions/0040-recorded-context-binding-history.md). This library-only slice
  does not implement HTTP history, owner receipt recovery or complete #100 acceptance.
- Add the opt-in immutable benchmark manifest library: bounded strict v1 parsing, separate
  gameplay/experiment/occurrence identities, exact mismatch reasons and keyed public references.
  Existing seed receipts can be associated with an immutable planned trial as `seed_receipt_bound`
  only when the declared protocol version and schema digest also match; this is offline consistency,
  not native reproducibility or hidden RNG verification. No runtime or legacy-record behavior
  changes. See [ADR 0021](decisions/0021-benchmark-manifest-foundation.md). Refs #121.
- Add scoped game-information v1 lookup consumption through the existing MCP port, a bounded
  typed agent tool loop, complete-source validation before projection, separate source/view
  identities and encrypted pinned replay archives. The opt-in mixed catalog preserves legacy
  profiles. Synthetic tool-loop and SQLite restart evidence do not claim native provider or
  exact-host execution; the old native Exo adapter remains terminal-decision-only.
  See [ADR 0039](decisions/0039-game-information-consumer.md). Refs #127.
- Add an opt-in Exo duplex lookup bridge and bounded native TypeScript tool registration,
  connecting the lookup agent API to the isolated pinned executor. See
  [ADR 0022](decisions/0022-exo-lookup-duplex-bridge.md). Refs #127.
- Expose the recorded context-owner binding for one workflow invocation over the authenticated
  management HTTP surface as a separately versioned, read-only projection. Same-subject scoped
  `workflow:read` is required; an unrecorded invocation, another subject, a missing scope and
  disabled retention are reported as distinct errors. Compatibility: additive read-only endpoint;
  no existing route, record, schema or resource bound changes. Does not establish current owner
  authority or receipt recovery. See [ADR 0023](decisions/0023-recorded-context-binding-http-projection.md).
  Refs #100.
- Add read-only recovery of already-issued context-control receipts. A caller whose delegated
  `pause`/`commit`/`resume` reply was lost can now look up the owner's recorded receipt by replaying
  the exact command instead of re-issuing it, gated on the binding advertising `receipt_recovery`.
  A recovered receipt must satisfy exact owner/invocation/binding/command identity before it is
  returned. Compatibility: additive, read-only, failing default port method; no schema, record,
  digest or resource bound changes. See
  [ADR 0024](decisions/0024-context-control-receipt-recovery.md). Refs #100.
- Expose the authoritative context owner's **current** association for one workflow run as the
  versioned read-only projection `ascension.harness.context-owner-association-view.v1` over
  `GET /v1/workflow-runs/{run_id}/context-owner-association`. The projected grants and epochs are
  owner assertions, not harness-issued authority; a binding for another run fails closed and an
  unattached owner stays explicitly unavailable. Compatibility: additive read-only route; the
  existing `ContextAssociation` route is unchanged. See
  [ADR 0025](decisions/0025-context-owner-current-association.md). Refs #100.
- Classify saved provider-session policies precisely against the **selected** adapter profile:
  `ProviderSessionPolicy::admit_for_profile` checks portable schema validity separately from the
  profile's executable ceiling and returns either a schema failure or a precise capability reason,
  never a generic invalid-policy error and never a silent clamp. Compatibility: additive; no policy
  field, schema, range or bound changes. See
  [ADR 0026](decisions/0026-provider-session-saved-policy-admission.md). Refs #95.
- Add bounded migration records for saved provider-session policies that are portable-schema valid
  but above the selected profile's executable ceiling: the exact saved bytes and violated limits are
  retained, and adoption requires explicit approval plus a caller-supplied target that is already
  within the executable ceilings — no value is ever silently clamped. Compatibility: additive,
  library-only; no policy field, schema, range or bound changes. See
  [ADR 0027](decisions/0027-provider-session-policy-migration.md). Refs #95.

- Record the **real pinned-Exo one-shot executor process oracle** against the shipped extension
  bytes and bind it mechanically: `docs/evidence/exo-executor-process-oracle-20260915.{md,json}`
  capture a reproduced run of the real pinned Exo runtime through `sts2-exo-bridge` →
  `sts2-exo-executor` with an original synthetic loopback model (27/27 cases, four correlated
  terminal decisions, retry containment to one egress, no provider, no game), the artifact manifest
  gains a `process_evidence` record, and workspace tests now fail closed when the extension module,
  the oracle source, or the pin inventory drifts from that record. Compatibility: additive;
  no schema, wire field, contract version, or runtime behaviour changes. See
  [the record](evidence/exo-executor-process-oracle-20260915.md). Refs #139.

- Tolerate a **transiently held** owner lease instead of reporting it as busy. `Lease::acquire` now
  retries the non-blocking lock attempt for a bounded interval, because an `flock` belongs to the
  open file description: a descriptor this process has already closed can still be held by a spawned
  child until it reaches `execve`, which can briefly outlive the owner that closed it. Exhausting the
  attempts still returns `Busy`, and the lock primitive, its exclusivity and its release on process
  death are unchanged. This removes the intermittent `restart: Busy` failure of the `exo_lifecycle`
  tests under parallel execution. Compatibility: no file-format, schema, range or bound change; a
  genuinely busy lease is now reported after about 160 ms (32 attempts, 5 ms apart) instead of
  immediately. See
  [ADR 0029](decisions/0029-owner-lease-transient-busy-retry.md). Refs #188.

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
  [ADR 0030](decisions/0030-context-owner-effective-limits-composition.md). Refs #95.

- Enforce the **selected** context-control limits that a binding advertises instead of only the
  harness maxima: `ContextRenderer::enabled_at_with_limits` refuses a draft that exceeds the
  advertised `max_items`, `max_notes`, `max_objective_bytes` or `max_context_bytes` with a precise
  error naming the limit, before any inference or retention. Compatibility: additive; `enabled`,
  `enabled_at` and `legacy` keep their signatures and behaviour, and no bound changes. See
  [ADR 0028](decisions/0028-selected-context-control-limit-enforcement.md). Refs #95.

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
  [ADR 0031](decisions/0031-runtime-exo-admission-gate.md). Refs #139.

- Keep a **cancel** pending as `NeedsOperator` while a live operation's settlement is still unknown,
  instead of stopping the episode and marking the run cancelled. `CommandKind::Cancel` reconciles
  first, and when reconciliation reports `ErrorClass::Unresolved` it returns
  `CommandOutcome::Pending` with reason `live_operation_unknown`, leaving the pending operation
  identity, the `NotStarted` cleanup state and the live session untouched so that an operator can
  still settle it. Compatibility: the cancel command, its revision guard and its response schema are
  unchanged, and a cancel at a settled revision behaves exactly as before. Refs #94.

- Renumber eight harness decision records whose numbers were each held by two different records, so
  every `ADR NNNN` label and `NNNN-*.md` link denotes exactly one decision. The moved records and
  every in-repo citation site were updated in the same change; no decision content changed. See
  [docs/decisions/README.md](decisions/README.md) for the old-to-new mapping. Refs #203.

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
  [ADR 0032](decisions/0032-inspected-admission-identity.md). Refs #139.

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

- Inspect the actual Exo launch configuration using the same loader as the bridge. Bind the
  configured extension, prompt, tool catalog, model route and configuration to the operator pins,
  require the package locator to match the configured executor, and use the shared artifact bounds.
  Envelope arguments now require `--run`, an absolute configuration path and its digest.
  Compatibility: breaking operator configuration, unchanged wire schemas. Unverified lifecycle
  capabilities remain an independent admission gate; no provider/native certification is claimed.
  See [ADR 0032](decisions/0032-inspected-admission-identity.md). Refs #139.

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
  [ADR 0032](decisions/0032-inspected-admission-identity.md). Refs #139.

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
  See [ADR 0046](decisions/0046-invocation-context-membership.md). Refs #106.

- Refuse recorded-run export explicitly on non-Unix platforms, where its descriptor-relative
  no-follow snapshot reader is unavailable, instead of preventing the whole harness from compiling.
  The Unix snapshot checks remain intact. This does not certify Windows runtime behavior.

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
  See [ADR 0046](decisions/0046-invocation-context-membership.md). Refs #106.

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
  [ADR 0047](decisions/0047-failed-command-event-classification.md). Refs #260.

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
  additive-compatible; see [ADR 0048](decisions/0048-context-owner-control-commands.md).
  Refs AI-Ascension/ascension-context-console#18.

- Promote the **runtime peer lane's MCP pin to a recovery-capable revision**. The lane declared MCP
  `f3b6eaa8`, which predates the `watchdog-recovery-v1` sideband profile the harness starts before it
  reads or reconciles a durable operation, so the lane's own recovery path was unreachable. The pin is
  now `587a53ce`, and the lane adds operator-only peer capability checks; the gateway pin is unchanged.

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
  [ADR 0052](decisions/0052-provider-session-effective-limits-route.md).
  Refs AI-Ascension/ascension-context-console#18.

- Execute independent read-only analyses of an admitted dynamic plan under an **owner-enforced
  in-flight cap** from `WorkflowLimits::max_parallel_analyses`, joined by node identity:
  `execute_plan_bounded` records each node `Settled`, `Failed` or `Unknown` in a `JoinedResult`
  whose `join_digest` is identical for every completion order, never dispatches a node whose
  declared input did not settle, and records an unwinding branch as `BranchLost` rather than
  stalling. `ParallelCap::SERIAL` keeps cap=1 compatible and `execute_plan` is unchanged; this is
  additive, and budget reservation, cancel/restart and browser branch state remain open. See [ADR 0049](decisions/0049-bounded-parallel-analysis-join.md). Refs #98.

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
  [ADR 0051](decisions/0051-logical-invocation-lifetime-consumption.md). Refs #111.

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
  See [evidence](evidence/exo-advertised-variant-negatives-20260918.md). Refs #141.

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
  [ADR 0054](decisions/0054-inference-profile-revision-adoption-scope.md)). Compatibility: additive;
  a versioned closed schema (`contracts/inference-profile/catalog.schema.json`) and one new route.
  Evidence is synthetic/component only. Refs #104.

- Remove an **orphaned `context_memory` source fragment** that made the repository impossible to
  check out on Windows. `crates/harness/src/context_memory/aux.rs` was 188 lines beginning inside an
  `impl` block and ending on a dangling attribute; nothing declared it, so no build, format, lint, or
  test ever read it, and its live counterparts are `approval.rs` and `authorizer.rs`. Because `aux`
  is a reserved Win32 device name with any extension, `git clone` on Windows stopped with
  `error: invalid path` and left an incomplete tree that could not be built. Compatibility: no
  behaviour change; the file was outside the module tree. Refs #281.

- Add opt-in Jev `--audit-dir` metadata sidecars with bounded, create-only Unix reservations,
  separate execution/input fingerprints, no raw prompts or action IDs, and no extra provider calls.
  Runtime stdout stays one decision; storage failures refuse it. Add a redacted paired reader and
  CI for the offline evaluation tests. Windows capture, native gameplay benefit and live paired
  orchestration remain unverified. See [capture documentation](../experiments/jev-evaluation/CAPTURE.md).
