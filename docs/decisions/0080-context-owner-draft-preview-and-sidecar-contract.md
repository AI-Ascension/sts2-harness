# ADR 0080: Context-Owner Draft, Preview, and Capability-Sidecar Contract

## Status

The coordinating owner selected this contract for T1 on 2026-10-04. That T1 change recorded the
owner/consumer decision only; it added no served route, durable record, Console adapter, sidecar, or
process acceptance. The current Harness #391 candidate implements the Harness owner routes and
durable draft, revision, preview, mutation-receipt, and draft-publication records described below on
context-control schema 3. Console #48 merged as `461bd9b71a25cd888f2c332e98c9e4e49294e390` and adds
trusted subject-bound grant ingress. The production Console-to-Harness owner adapter and encrypted
durable intent containing the exact original mutation request for recovery remain pending, as do
the separate-process Console R3 response wire and recovery and the independently trusted ADR 0021
sidecar join. Harness #391 and Context Console #18 remain open through their remaining criteria.
This is not account-independent GitHub approval or native, provider, deployment, or production-host
evidence.

## Context

At the T1 source baseline, the Harness `ContextOwnerPort` in
`crates/harness/src/management/context_owner_support.rs` supported binding and association reads,
source status, control and control-receipt operations, immutable source publication, and source
adoption. The management `PUT /v1/workflow-runs/{run_id}/context-sources/{source_id}` publishes an
immutable `ContextSourceDocument`; `POST .../context-sources/{source_id}/adopt` makes an explicit
final revision commit. The current #391 candidate extends that owner boundary with the durable,
run-scoped draft, item, revision, preview, exact mutation-receipt, and draft-publication operations
described below. Source status still exposes metadata, not content.

`ContextDraft` (`crates/harness/src/context_control/types.rs`) stores exact item references in
`selected_items`, `notes`, and `objective`, plus selected item IDs in `pinned_item_ids`.
`ContextNote` contains a `ContextItemRef` and attribution; it does not carry note text. A
`ContextSourceDocument` pairs the draft with the actual `ContextItem` bytes. The existing managed
render path resolves references against an owner-supplied item registry and checks exact reference,
content digest, scope, protection, and expiry (`context_control/render/admission.rs` and
`membership_resolution.rs`). The durable source store retains encrypted immutable snapshots. The
current T2 implementation also persists owner-scoped drafts and derives eligible-item projections
from stored owner records; a caller-supplied reference still cannot create eligibility.

The owner already serves `POST
/v1/workflow-runs/{run_id}/context-control-receipts/lookup` for recorded pause, commit, and resume
receipts. It requires the exact original `ContextControlCommand`; it does not make a lookup by key
alone equivalent. Harness owner receipt v2, its identity fields, and its existing encryption/AAD
remain unchanged. Console's `HarnessOwnerPort` has a key-only recovery convenience, so a real
adapter must persist and replay the exact original owner command under its request identity.

Console ADR 0021 allows an optional internal `EffectiveLimitRecord` on an accepted
`OwnerReply` for capability operations. The Harness producer record
`ascension.harness.effective-limits.v1` is served by
`GET /v1/workflow-runs/{run_id}/provider-session-effective-limits`; it is bounded metadata from the
served provider-session descriptor, not a receipt for a particular accepted Console owner
operation. The context-memory effective-limits route returns
`context_memory_record_unavailable` because the served owner has no memory corpus. The
`/provider-sessions` live-inspection route remains unavailable under the shipped serve/run process
split selected by Harness #795 option C.

## Decision

### Harness owns the durable scoped operations

Harness remains the sole durable authority. The current T2 candidate provides a typed owner port
and authenticated management operations for:

- listing eligible owner items for one run and draft, with metadata by default and item bytes only
  when the caller also has `workflow:context:content:read`;
- creating and reading a draft, applying a compare-and-swap patch, and listing or reading the
  resulting durable revisions;
- creating and reading an owner-rendered preview; and
- recovering a durable draft/preview mutation receipt from the caller's exact original request.

The current Harness route family serves `GET .../context-owner-items`, `GET/POST
.../context-owner-drafts`, `GET/PATCH .../context-owner-drafts/{draft_id}`, paginated `GET
.../context-owner-revisions`, direct `GET
.../context-owner-revisions/{revision_id}`, `POST
.../context-owner-drafts/{draft_id}/previews`, `GET
.../context-owner-previews/{preview_id}`, and `POST
.../context-owner-mutation-receipts/lookup`, all under
`/v1/workflow-runs/{run_id}`. The current source also serves `GET
.../context-owner-published-sources`, `POST
.../context-owner-drafts/{draft_id}/publications`, and `POST
.../context-owner-draft-publication-receipts/lookup` for explicit immutable draft publication and
its exact receipt recovery. These are Harness owner routes; they do not establish a Console
production adapter or separate-process Console/Harness acceptance.
The item-list request may ask for content explicitly; the owner must authorize and bound that
projection independently. Draft, revision, preview, and receipt projections contain references and
metadata only; raw owner item bytes, including new note/objective bytes, are returned only by the
eligible-item content operation after its separate grant. That grant does not permit prepared
provider-input display, capture enablement, or arbitrary content references. Revision lists are
paginated and bounded, while direct revision lookup remains available when a Console-local
reference index evicts an ID. The current implementation has no delete operation.

Use new closed owner envelopes, independently versioned from the existing Console schemas and the
inner `ascension.context-control.draft.v1` record. Initial names are
`ascension.harness.context-owner-draft.v1`,
`ascension.harness.context-owner-preview.v1`, and
`ascension.harness.context-owner-mutation-receipt.v1`. A draft envelope binds owner, workflow run,
invocation, current binding ID and digest, authenticated subject, the full episode/agent/boundary and
epoch tuple, base revision, draft ID/version, timestamps, and the inner draft. A preview is bounded
metadata binding that draft/version and base revision to the current boundary, exact render/manifest
digest, blockers, effect class, and expiry; it never substitutes for or returns prepared provider
input bytes.

An eligible-item response is derived only from the current authenticated owner's durable
run-scoped item/source records and binding. `ContextSourceDocument.items` contains bytes for an
immutable source snapshot, while `ContextOwnerRenderRequest.registry` remains an in-process render
input; neither by itself is a public eligible-item catalog. The T2 owner lookup resolves stored
owner-controlled records and binds each returned reference to its exact source identity/revision. If
the owner cannot resolve the item from that authority, it returns unavailable/unsupported. A
caller-supplied item ID, version, hash, or `ContextItemRef` never creates eligibility: the owner
resolves stored bytes and rechecks the digest, scope, protection, and expiry before use.

Draft patches preserve the following existing meaning:

- **Include/exclude:** include only owner-resolved, in-scope eligible references. Exclusion can
  remove only a selected reference; protected owner prerequisites cannot be excluded. An invalid,
  foreign, expired, revoked, or digest-mismatched reference is refused rather than silently
  admitted.
- **Pin/unpin:** pinning is restricted to an included item. Effective pins are revalidated against
  the final included set; an exclusion cannot leave an item pinned. Pins cannot bypass item bounds,
  owner prerequisites, or the effective membership policy.
- **Notes:** the stored `ContextNote` continues to reference an owner-held item, and its attribution
  is derived from the authenticated subject, not the request's `attributed_to` or `author_ref`.
  Newly authored note text is stored only in the owner's encrypted draft/item record, with the
  current 16-note and 4-KiB-per-note bounds, any lower owner-advertised limits, and the existing
  total-context bound. The separate explicit draft-publication operation snapshots the final item
  bytes into the existing encrypted immutable-source store; it does not automatically adopt that
  source as the active revision.
- **Objective:** the draft continues to reference a bounded owner-held item; the current render
  path enforces the 512-byte objective bound and any narrower advertised limit. Creating,
  replacing, or removing it requires the separate `workflow:context:objective:edit` authorization.
  Ordinary context edit, content read, immutable source publication, and control authority do not
  grant objective edit.

Newly authored note/objective bytes remain in owner-controlled encrypted records with the existing
owner retention and expiry policy. They are never cached as authoritative Console state or copied to
logs. The current schema-3 SQLite migration supports authenticated v1-to-v3 and additive v2-to-v3
upgrades, preserving existing encrypted source and receipt bytes and AAD; new record types use
purpose-specific AAD. It does not extend retention or capture defaults.

### Grants and actor binding

Every operation is re-authorized at the owner against the same authenticated subject that the
current production binding requires; preserve the `entry.actor == AuthContext.subject` check. The
scope mapping is:

| Operation | Harness scope |
| --- | --- |
| metadata, bounded list/read, revision and exact receipt recovery | `workflow:read` |
| eligible item bytes | `workflow:context:content:read` |
| create/patch drafts, include/exclude, pin/unpin, notes, and create previews | `workflow:context:edit` |
| set/replace/remove objective | `workflow:context:objective:edit` |
| existing pause, commit, and resume commands | `workflow:control` |
| existing immutable source upload | `workflow:content:write` |

`workflow:context:content:read` reveals only the bytes of owner-advertised, bounded, eligible items
that the current run/binding and existing protected-content, expiry, and retention policy admit. It
does not widen existing immutable-source publication behavior or the legacy Console content
permission descriptor.

Console's independent metadata, content, edit, objective, commit, pause, and resume grants remain
separate at its boundary. The Harness adapter forwards only the operation authorized by the
matching grant. Existing `workflow:*` keeps its accepted behavior for every required scope; this
decision does not narrow or reinterpret it. A narrow ordinary edit grant, `workflow:content:write`,
or `workflow:control` alone does not authorize objective edits. Objective edits require the
separate narrow scope or an existing authenticated `workflow:*` grant under unchanged wildcard
semantics.

The inspected T1 Console facade boundary lacked an authenticated subject on `CapabilityGrant`, and
its `x-principal` field came from a caller header. Console #48's merge at
`461bd9b71a25cd888f2c332e98c9e4e49294e390` adds trusted subject-bound grant ingress. Harness T2
routes authorize the actor supplied by the Harness authentication context. The production
Console-to-Harness adapter that forwards those grants and the protected owner credential, encrypted
durable intent containing the exact original mutation request, and the separate-process Console R3
response path remain pending. A caller-controlled `x-principal` remains insufficient proof of
identity; subjectless legacy grants must fail closed for new actor-bound owner operations. Keep
demo/legacy composition separately labeled. A service-level protected auth reference may access
only the owner subject it actually authenticates; it is not a substitute for per-request identity.

Draft/preview write receipts are distinct from `ContextControlReceipt` v2. A successful mutation
commits its record change and terminal receipt atomically in the Harness owner store. The receipt
binds owner/run/subject/binding and full boundary/epoch identity, operation, caller request ID,
canonical payload digest, and stable result ID. An exact same-scope/same-ID/same-digest retry returns
the same stored result. Same ID with another digest returns a generic conflict without disclosing
whether another actor has a record. Recovery replays the exact original request identity and reads
only the stored terminal result; it never re-applies a write. If a reply is lost after a write could
have reached the owner, the consumer reports write state unknown until that exact receipt can be
recovered; it does not retry with a changed command or call the operation again. A known refusal
before owner reservation remains a no-effect result.

For current pause/commit/resume, keep the v2 owner receipt and exact command shape unchanged.
Console must durably retain the exact command fields (operation, request/idempotency identity,
expected version, boundary and revision/preview/approved digests as applicable) and submit them
unchanged to the owner lookup after restart. A key-only lookup or reduced public Console receipt is
not equivalent owner correlation. Receipt lookup is a read of historical evidence and does not
require the current binding to remain active; new writes always recheck current actor, binding,
CAS, and epochs.

### ADR 0021 capability sidecar and unsupported records

The Console sidecar remains internal to accepted `OwnerReply` capability operations and its
existing closed public schemas do not change. Admit a sidecar only when a real adapter can prove all
of the following from trusted configuration and the exact accepted owner response: the operation
was `memory.capabilities` or `provider_session.capabilities`; the record uses the independently
expected schema and descriptor digest; the authenticated owner and exact project/run/episode/agent
scope match; and the source, owner revision, active binding, and epoch match the independently
trusted current values. A record embedded in `OwnerReply.value`, an untrusted caller field, a copied
fixture, or a producer response without this join is not authority. The Harness provider-session
effective-limits route is candidate metadata only: on its own it does not prove the exact Console
operation, source receipt, complete scope, or owner epoch. Until a production adapter proves the
join, omit the sidecar and return the existing typed unavailable/unknown result.

Keep `context_memory_record_unavailable` for the absent memory corpus. Keep live
`/provider-sessions` inspection unavailable under the selected Harness #795 option C topology; do
not create a serve-process broker registry or infer a session from metadata. Neither limitation
changes the Console ADR 0021 trust rules.

## Compatibility and current implementation boundary

The 2026-10-04 T1 selection changed no served Harness route or record. The current #391 source adds
the selected owner envelopes and authenticated Harness routes without changing the existing Console
public schemas, control-receipt v2, protocol artifacts, producer pins, defaults, or retention rules.
Context-control SQLite schema 3 is a forward-only migration: fresh stores use v3 and authenticated
v1/v2 stores upgrade to v3 additively. Older binaries that reject v3 must not open a migrated store;
do not downgrade it or rewrite authenticated bytes. Snapshot the encrypted store before migration.
Any rollback to an older binary requires restoring its matching pre-migration snapshot; this ADR
requires that backup/rollback path but does not claim the operational procedure is implemented. The
owner still uses the existing binding, source, render, and receipt seams.

The current Harness candidate includes durable eligible-item, draft/CAS, revision, preview, exact
mutation-receipt, and explicit draft-publication operations. Focused source tests cover stored-byte
eligibility; metadata versus content grants; include/exclude and protected prerequisites; pin/unpin;
attributed notes and bounds; objective denial without its separate grant; draft CAS and base-revision
drift; preview from the production render path; objective success with an owner-verified grant; exact
idempotent recovery; and generic same-ID/different-payload conflict. These are source-level and
served owner tests, not separate-process Console/Harness acceptance.

Console #48 now supplies trusted subject-bound grant ingress. The outbound Harness owner adapter,
encrypted durable storage and replay of exact mutation requests, Console R3 response wire and
cross-process recovery remain to be implemented and accepted; the independently trusted ADR 0021
sidecar join is also pending. Keep the protected owner credential inside protected configuration;
never put it in request bodies, URLs, owner records, browser storage, or logs. Populate the sidecar
only after the exact trusted owner, descriptor, operation, source, revision, binding, and epoch join
succeeds. Native, provider, deployment, and production-host evidence remain separate.

Acceptance requires Harness and Console as separate real processes with the production auth ingress,
not `RecordingOwner`, `NoopOwner`, a synthetic `ControlPlane`, or fixtures as authority. Exercise
eligible items both metadata-only and with content grant; create/list/get a draft; select/include,
exclude, pin/unpin and author a note; deny objective changes without its separate grant; allow an
objective change with its separate verified grant; preview, commit while held, and explicitly resume.
Include spoofed-header, subjectless legacy-grant,
different-subject, foreign/stale scope, protected-item, expired/digest-tamper, ordinary-versus-
objective, same-ID/different-digest, lost-reply, and independent owner/Console restart cases. After
restart recover the exact original receipt without reapplying the effect; an old-epoch write is
denied while historical exact-receipt recovery remains readable to the same actor. Mutate sidecar
descriptor, scope, source, revision, owner, and epoch inputs and verify fail-closed omission. Verify
memory remains unavailable and `/provider-sessions` remains structurally unsupported. These tests
establish service-process behavior only; native game, provider, deployment, and production-host
evidence remain separate.

## References

- Harness source baseline: `44e63cf3222ec6f73d9156a48387f446fc689609`; the cited owner files are
  unchanged from `982414ea9f2383bc92806660d30e946a0d8030d1` across the #830 merge. See
  `context_owner_support.rs`, `context_owner_source.rs`, `http_routes_run.rs`,
  `context_control/types.rs`, `context_control/store_schema.rs`, and
  `production_context_owner/source.rs`.
- Context Console source baseline: `cde74812cfbe93faf47c6cb3b390fc5a4c807769`; see
  `harness_facade.rs`, `docs/decisions/0021-owner-capability-sidecar.md`, and its authenticated
  permission/owner-port contract.
- Scope: [Harness #391](https://github.com/AI-Ascension/sts2-harness/issues/391),
  [Context Console #18](https://github.com/AI-Ascension/ascension-context-console/issues/18), and
  [Harness #795 option C](https://github.com/AI-Ascension/sts2-harness/issues/795).
