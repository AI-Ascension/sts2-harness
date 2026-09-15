# Changelog

All notable user-visible or operational changes to this project are documented here.

The project follows Semantic Versioning once versioned releases begin. Foundation work does not
claim a released harness version or runtime compatibility.

## Unreleased

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
  See [ADR 0021](docs/decisions/0021-game-information-consumer.md). Refs #127.

- Add the opt-in immutable benchmark manifest library: bounded strict v1 parsing,
  separate gameplay/experiment/occurrence identities, exact mismatch reasons and
  keyed public references. Existing seed receipts can be associated with an immutable
  planned trial as `seed_receipt_bound` only when the declared protocol version and schema
  digest also match; this is offline consistency, not native
  reproducibility or hidden RNG verification. No runtime or legacy-record behavior changes.
  See [ADR 0021](docs/decisions/0021-benchmark-manifest-foundation.md). Refs #121.

- Add opt-in bounded SQLite history for context-owner bindings, committed atomically with
  command results and read by original invocation with current scoped, same-subject permission.
  Historical grants and epochs never authorize current control or claim a restored owner.
  Public JSON schemas and current-cursor association stay unchanged; Rust `CommandApplication`
  constructors must supply the new optional `context_binding` field. See
  [ADR 0022](docs/decisions/0022-recorded-context-binding-history.md). This library-only slice
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

- Add opt-in Exo owner persistence with a separate encrypted broker journal, lifetime owner lock,
  authenticated send/result fences, conservative restart handling, and explicit v1 cutover.
  Existing execution-store reservations and result bytes retain their ownership. See
  [ADR 0020](docs/decisions/0020-exo-owner-journal-and-single-use-send.md).
  Focused source/recording-fixture and local process coverage passed 37 test functions at
  `29d256c`; this is not native Exo or full-runtime acceptance. Cancellation, native
  reconciliation, qualified accounting, containment and episode admission remain gated. Refs #142.

- Add the owned `sts2-exo-bridge` single-turn process entrypoint and an isolated, exact-pinned
  real Exo embedding package. Strict standard/fresh requests retain their complete catalog and
  constraints; correlated output is independently parsed with no fallback. The tool-free extension
  forwards at most one model request and records denied upstream SDK retry attempts. Add
  `ExoAdmittedTransport` for full-preflight envelope handoff with explicit host identities.
  Compatibility: additive opt-in source/process path; full runtime admission, map/expert/recovery,
  durable lifecycle, actual provider/game execution and replay remain separately gated. Refs #141.

- Align the reviewed Console and Studio v3 effective-limit consumer pins, require exact
  repository-owned CI references, and verify candidate producer-library bytes against both
  unchanged consumer fixtures before admission tests. Validate actual static YAML checkout
  steps with pinned `yaml-rust2` 0.13.0; shell text, ambiguous mappings and conditional/inert
  steps cannot establish a consumer pin. Preserve the separate Studio workflow-owner
  regression; effective-limit evidence remains synthetic only.

- Add producer-generated context-control catalog fixtures for descriptor minima, restricted
  values, global ceilings, disabled metadata and digest consistency. A catalog sealing helper
  reuses the existing v1 encoding. No wire fields, limits or selected-limit execution behavior
  change; consumer adoption and rendering/journal enforcement remain separate. Refs #95.

- Add an opt-in authenticated durable memory-policy owner: retain exact saved encodings, require
  target-bound approval for atomic adoption, and load the adopted policy for actual local memory
  selection. Restart requires explicit revalidation; stale grants/profiles/revisions fail closed.
  The separate encrypted bounded store preserves history and idempotent receipts. This is the
  memory-only component slice of #95, not session migration, browser/production wiring or #119.

- Publish the machine-readable `ascension.harness.effective-limits.v1` classification record so every
  advertised context-memory and provider-session value is classified as schema-valid versus
  executable for the selected profile, with machine-readable unavailable reasons. Add a
  producer/consumer pin and digest conformance matrix (`contracts/effective-limits-pins.json`) that
  recomputes producer digests and fails closed on drift, tampering, a stale adoption label, an
  unrecorded surface, or a consumer that still validates a `v1` capability schema. Both recorded
  consumers (Context Console, Studio) remain `pending`, so neither can present a value the runtime
  rejects. Deterministic offline tests only; consumer adoption, native, and provider evidence remain
  `unverified`.

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

- Define the harness-owned `sts2-exo-bridge-v1` contract and freeze the candidate Exo source
  manifest. The closed capability/preflight and request/turn envelopes enforce independent
  identity, bounds, UTF-8/framing, correlation, terminal-decision, cancellation, and EOF rules;
  deterministic fixtures cover rejection vectors. The nine-commit upstream review found no native
  machine executor hook, so package, model, extension, native connectivity, and gameplay evidence
  remain `unverified`; see ADR 0017.

- Require a closed `ExoRestrictedProfile` in the trusted Exo configuration for issue #140.
  Admission now fails closed before inference on any non-empty/unreviewed model tool, duplicate or
  invalid tool names, unsafe or overlapping private state/cache/temp roots, unbounded
  quota/retention, or permissions other than `0o700`, with canonical catalog and profile digests.
  The reviewed model tool allowlist is intentionally empty; TypeScript dispatch, OS containment, and
  native private-state enforcement remain follow-up work.

- Add a pinned runtime-peer CI lane. It builds the candidate harness against
  immutable gateway and MCP executable peers, uses a bounded synthetic mod HTTP
  endpoint only as downstream, and runs positive plus foreign-identity and
  malformed-envelope rejection cases. Startup and cancellation cleanup
  regressions run in the same lane. This is source-derived synthetic process
  composition evidence, not game-host, provider, or release qualification.

- Reject incomplete exact checkpoint manifests, inconsistent dependency sizes and payload
 identities, and source/destination profile mismatches before session admission. Bound reads of
 persisted exact artifacts to 16 MiB. These checks establish component integrity, not live restore
 certification; see ADR 0016.

- Add the bounded `coop-native-v1` cohort coordinator and canonical-peer attribution safety
  correction. A returned observation can be attributed only when its sole local peer equals the
  originally scheduled canonical actor and its instance/session/lease/epoch fence matches the
  original operation. Mismatches retain pending or unknown operations for same-operation
  reconciliation. The frozen artifact, wire profile, digest, and producer goldens are unchanged;
  route credentials remain outside harness records. This is source/component evidence only, not
  native multiplayer transport, settlement, or release compatibility.

- Digest all serialized Runtime-v3 telemetry lineage identities with domain-separated SHA-256
  values while retaining raw trace lineage only for private OTLP topology derivation. Exporter
  tests cover raw prompt, model-output, credential, path, and proprietary-text sentinels through
  the full serialized OTLP envelope. Backend queries must use the deterministic digest while raw
  mappings remain access-controlled local run evidence; collector/backend and live evidence remain
  unverified.

- Tighten optional seeded-receipt replay admission before `EpisodeRunner` construction. A receipt
  preamble now requires exact current seed configuration and original operation/fence/context
  equivalence, a fresh canonical-seed run-start witness, and a closed MCP wrapper/result chain.
  This is deterministic source/component validation only; it does not execute replay or invoke a
  provider, MCP server, gateway, or host.

- Persist validated Runtime-v3 action-wait settlement against its original durable operation
  before admitting another model decision. Previously the host could settle the action while
  the durable store retained `unknown`, causing the next decision to fail with a misleading
  provider-malformed error. Unresolved waits and invalid witnesses retain durable uncertainty.
  Synthetic runtime tests cover decision admission, repeated waits, and database reopening;
  native campaign validation remains a separate gate.

- 2026-09-10: Add the additive `seeded-run-v1` transport handoff at harness main
  `3926e5a30ab569612e67d2dfdc6542f1391e95d7`. It validates a bounded contiguous seed plan and
  context-digest-bound standard Ironclad selection, establishes a generation fence, creates one
  durable reservation before `start_seeded_run`, and reconciles unknown starts with the same
  operation ID. The copied `sts2-protocol/seeded-run-v1` artifact has schema digest
  `5c659f344be78f84e8d783986925d462714f933cac95d18943358992f7d3e2b8`, aligned with protocol main
  `d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404`. Source/component and artifact checks do not establish
  native seed settlement, profile/save isolation, gameplay, deployment, or release compatibility.

- 2026-09-10: Add the `coop-native-v1` consumer boundary as a source/component integration. The
  strict parser consumes the copied accepted artifact, recognizes recovery requests that use the
  bodyful `recovery_response` shape, enforces peer-token uniqueness and host-generation relations,
  and retains unknown mutations for same-operation reconciliation. Deterministic tests cover all
  seventeen producer goldens and malformed peer, effect, receipt, and recovery mutations. This
  does not add MCP/HTTP/gateway transport or claim a live native multiplayer session; peer admission,
  host legality, settlement, checksum convergence, rejoin, deployment, and release compatibility
  remain `unverified`.

- 2026-09-10: Fence unknown recovery receipts to the observed host generation. The accepted
  same-generation receipt exception is limited to pending rejoin recovery; reconcile and unresolved
  receipts retain a null after-generation until a settled response.

- Record visible Astra-controlled v0.107.1 campaigns and fresh process replays through the full
  harness → MCP → gateway → mod path: Windows reached Defeat with 333 settled actions; Linux
  reached Defeat with 431 after one controller restart following a catalog-read failure. These
  are bounded fixture records; model-played Victory, all campaign branches, native multiplayer,
  and broader compatibility remain unverified. See `docs/evidence/seeded-astra-campaign-20260906.md`
  and `docs/evidence/linux-seeded-campaign-20260906.md`.

- Consume the coordinated Runtime-v3 continuation schema with argument-free proceed,
  confirm-selection and cancel-selection actions; reject mixed revisions and extra arguments.

- Add OpenAI Astra combat decisions through authenticated, ephemeral Codex calls. Provider
  bridges describe their identity so live-run manifests distinguish OpenAI and Ollama.

- Preserve the host's visible seed by default for repeatable calls and replay. Explicit
  `STS2_EXO_FORWARD_VISIBLE_SEED=false` still supports seed-blind experiments.
- Add an opt-in real combat demo with a bounded Ollama provider bridge and host settlement
  records. Preserve unknown gameplay receipts for same-operation reconciliation through MCP.
- Release router bindings rejected for mismatched run or episode identity and surface cleanup
  failures through the routing error boundary.
- Split the independent Runtime-v2 coordinator and process trace from PR #7; the bounded
  Runtime-v3 card-play probe remains outside this change. Frozen artifact bytes are unchanged.
- Document coordinated explicit gateway/MCP session configuration and verify independent session
  identities and the selected runtime profile reach a spawned fake MCP process.

### Added

- Complete the frozen Runtime-v1 consumer checksum inventory and golden messages; check both
  frozen runtime inventories in CI without changing existing wire schemas or manifests.

- A dated, evidence-labeled expert-state information-architecture research specification covering
  fair-play observation, the proposed atomic-state/action inventory, recovery, evaluation, and
  patch drift. It is explicitly not full-game or gameplay-proof evidence.

- A generated expert-state requirements package with 131 candidate states, typed observation/action/
  transition inventories, closed JSON schemas, synthetic fixture classes, per-state Markdown, and
  Mermaid sources. The package remains target-build validation material, not runtime support.

- A bounded Runtime-v2 multi-instance coordinator seam with four-lane registration, explicit
  identity isolation, fair serial dispatch, global/per-instance backpressure, queued cancellation,
  active-work reconciliation reporting, and sanitized snapshots. This is component evidence only.
- Propagated the independently configured Runtime-v2 MCP session through gateway allocation,
  spawned MCP configuration, request correlation, and lease release. The gateway session remains
  the frozen protocol-envelope identity; gateway and MCP session values must be distinct.

### Safety corrections

- Generate UUIDv4 operation identities at production action creation and retain the same identity
  through uncertain dispatch and recovery reconciliation. Preserve the authoritative host state ID
  without substitution; the frozen Runtime-v3 gameplay artifact is unchanged. Focused source and
  component tests cover recovery-boundary acceptance, restart uniqueness, and conflicting action
  reuse. See [ADR 0012](docs/decisions/0012-operation-identity-at-creation.md).

- Validate historical recovery against retained canonical action bytes, the requested original
  authority, terminal ticket and operation-specific witness. Reconcile unresolved lookups without
  gameplay polling; accept retained terminal gateway states and bounded padded/unpadded action
  encodings. Missing evidence remains unresolved. This is synthetic consumer validation, not
  cross-boot recovery or a release-set claim.

- Require a `released` status after runtime lease cleanup; a successful HTTP exchange alone
  no longer counts as confirmed release.

- Retain unknown operations in their serial instance lane until explicit reconciliation.
- Bound MCP and loopback gateway exchanges end to end, reap owned MCP children, and reject
  mismatched responses without printing downstream payloads or inheriting unrelated credentials.
- Validate exact Runtime-v2 response contracts and retain only numeric legacy gameplay trace fields.

- Require recovered transitions to match the complete dispatched action, and reconnect failed MCP
  transports only for bounded recovery reads while retaining operation identity. Attempt fenced
  lease cleanup when an allocation response is lost or invalid.
- Split Exo request validation, fair-play schema rules, decision replay, and evaluation report
  projection into cohesive modules within ordinary policy budgets; remove handwritten exemptions.

- The bounded Runtime-v3 episode state machine, semantic action ledger, transition barrier and
  recovery ports, and strict Exo fair-play decision adapter.
- A bounded complete-run coordinator that routes every declared playable surface through the
  current host legal-action catalog and independently verifies transition settlement.
- An operator-owned direct Exo process transport with bounded stdin/stdout, timeout, environment
  allowlisting, and fail-closed shutdown behavior.
- An offline Exo configuration example and explicit `unverified` live-connectivity status.
- Full-run routing coverage for setup, map, combat, reward, shop, event, rest, selection, and
  separate victory/defeat terminal observations.
- Bounded typed decision records, memory, replay/evaluation metrics, cooperative synchronization
  gates, and quarantined M10 build/patch manifest preparation.
- The bounded `sts2-harness-runtime` coordinator, `runtime-v1` artifact copy, real MCP/gateway
  process path, stale-generation oracle, sanitized trace, and component evidence record.

- A dated authorized-host integration record confirming the complete bounded coordinator-to-STS2
  runtime probe, visible effect witness, stale-generation rejection, and reversible cleanup.

- Repository governance, policy-as-code, workflow, licensing, security, and release foundations.
- Harness-specific ownership, dependency, protocol-repository, compatibility, and provenance decisions.
- Documentation for multi-instance coordination, model/provider ports, episodes, trajectories,
  replay, scoring, evaluation, and artifact lineage.
- A target-owned Rust harness package with explicit routing, provider, record, replay, artifact, and
  shutdown ports plus deterministic fake-boundary tests.
- A copied release-like `sts2-protocol/poc-v1` artifact, deterministic five-boundary fake runner, and
  [`MINIMAL_POC_REPORT.md`](MINIMAL_POC_REPORT.md) with the canonical 15-event trace.

### Changed

- Standardized historical POC and Runtime-v2 fake evidence labels to `confirmed`, retaining
  their deterministic-fake scope, original dates, trace bytes, digests, and unverified live lanes.

- Attempt fenced allocation cleanup when the runtime coordinator cannot accept an allocation
  response, preserving the configured trace fence and reporting release failures explicitly.
  Deterministic fake-boundary coverage does not establish live cleanup behavior.

- This package-preparation entry covered no live providers, game access, gateway lease ownership,
  MCP framing, game rules, scoring, dataset export, or training integration; those concerns remain
  outside that historical wave. Later dated campaign records are scoped separately.
- Exo revisions are now required to be exact non-zero lowercase commit hashes; the checked-in
  example uses the reviewed public audit revision and does not claim live connectivity.

### Deprecated

- Nothing.

### Removed

- Nothing.

### Fixed

- Nothing.

### Security

- No provider, game, profile, save, credential, model, or dataset access was added.
