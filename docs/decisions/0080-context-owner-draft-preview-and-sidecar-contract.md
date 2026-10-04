# ADR 0080: Context-Owner Draft, Preview, and Capability-Sidecar Contract

## Status

Proposed for implementation. On 2026-10-04 the coordinating owner selected the contract below for
T1 review. This record describes future owner and consumer behavior; the PR that adds it is
documentation only. It adds no served route, durable record, Console adapter, sidecar, or process
acceptance result. Harness #391 and Context Console #18 remain open through implementation and
separate-process acceptance. This is not account-independent GitHub approval or native, provider,
deployment, or production-host evidence.

## Context

The Harness `ContextOwnerPort` in `crates/harness/src/management/context_owner_support.rs` currently
supports binding and association reads, source status, control and control-receipt operations,
immutable source publication, and source adoption. The management `PUT
/v1/workflow-runs/{run_id}/context-sources/{source_id}` publishes an immutable
`ContextSourceDocument`; `POST .../context-sources/{source_id}/adopt` makes an explicit final
revision commit. The source status exposes source metadata, not content. None of these operations
is mutable Console draft CRUD, revision history read, or preview create/read.

`ContextDraft` (`crates/harness/src/context_control/types.rs`) stores exact item references in
`selected_items`, `notes`, and `objective`, plus selected item IDs in `pinned_item_ids`.
`ContextNote` contains a `ContextItemRef` and attribution; it does not carry note text. A
`ContextSourceDocument` pairs the draft with the actual `ContextItem` bytes. The existing managed
render path resolves references against an owner-supplied item registry and checks exact reference,
content digest, scope, protection, and expiry (`context_control/render/admission.rs` and
`membership_resolution.rs`). The durable source store retains encrypted immutable snapshots. That
does not establish a persisted mutable-draft registry or an HTTP `eligible_items` operation today.

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

Harness remains the sole durable authority. Add a typed owner port and authenticated management
operations for:

- listing eligible owner items for one run and draft, with metadata by default and item bytes only
  when the caller also has `workflow:context:content:read`;
- creating and reading a draft, applying a compare-and-swap patch, and listing or reading the
  resulting durable revisions;
- creating and reading an owner-rendered preview; and
- recovering a durable draft/preview mutation receipt from the caller's exact original request.

The candidate route family is `GET .../context-owner-items`, `GET/POST
.../context-owner-drafts`, `GET/PATCH .../context-owner-drafts/{draft_id}`, paginated `GET
.../context-owner-revisions`, direct `GET
.../context-owner-revisions/{revision_id}`, `POST
.../context-owner-drafts/{draft_id}/previews`, `GET
.../context-owner-previews/{preview_id}`, and `POST
.../context-owner-mutation-receipts/lookup`, all under
`/v1/workflow-runs/{run_id}`. These are proposed routes, not routes that the current server serves.
The item-list request may ask for content explicitly; the owner must authorize and bound that
projection independently. Draft, revision, preview, and receipt projections contain references and
metadata only; raw owner item bytes, including new note/objective bytes, are returned only by the
eligible-item content operation after its separate grant. That grant does not permit prepared
provider-input display, capture enablement, or arbitrary content references. Revision lists are
paginated and bounded, while direct revision lookup remains available when a Console-local
reference index evicts an ID. The first implementation has no delete operation.

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
run-scoped item/source records and binding. In today's implementation, `ContextSourceDocument.items`
contains the bytes for an immutable source snapshot, while `ContextOwnerRenderRequest.registry` is
an in-process render input; neither is a public eligible-item catalog. T2 must add a durable owner
lookup over those owner-controlled records (and any newly authored items) and bind each returned
reference to its exact source identity/revision. If the owner cannot resolve the item from that
authority, it returns unavailable/unsupported. A caller-supplied item ID, version, hash, or
`ContextItemRef` never creates eligibility: the owner must resolve the stored bytes and recheck the
digest, scope, protection, and expiry before use.

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
  total-context bound. The immutable
  source published at commit carries the final item bytes in its existing encrypted source record.
- **Objective:** the draft continues to reference a bounded owner-held item; the current render
  path enforces the 512-byte objective bound and any narrower advertised limit. Creating,
  replacing, or removing it requires the separate `workflow:context:objective:edit` authorization.
  Ordinary context edit, content read, immutable source publication, and control authority do not
  grant objective edit.

Newly authored note/objective bytes remain in owner-controlled encrypted records with the existing
owner retention and expiry policy. They are never cached as authoritative Console state or copied to
logs. The new schema migration preserves all existing encrypted source and receipt bytes and their
AAD, and uses purpose-specific AAD for new record types. It does not extend retention or capture
defaults.

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

At the inspected Console facade boundary, `CapabilityGrant` has no authenticated subject, the
in-process `x-principal` field is copied from a caller header, and the checked source does not
provide a production token issuer or verified-principal ingress. T2 must bind the subject from
trusted protected authentication to the grant and owner credential, reject a mismatching
`x-principal`, and make subjectless legacy grants fail closed for new actor-bound production owner
operations. Keep demo/legacy composition separately labeled. A service-level protected auth
reference may access only the owner subject it actually authenticates; it is not a substitute for
per-request identity.

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

## Compatibility and first implementation partition

This decision changes no currently served Harness route, Harness record, Console schema, receipt,
fixture, protocol artifact, producer pin, default, or retention rule. Implement the owner operations
as additive Harness-only envelopes and routes; the boundary has one named production consumer, so
do not move it into `sts2-protocol`. The new scopes/envelopes are additive contracts, but the v1 to
v2 owner-store migration is one-way for older binaries: an old reader that rejects the newer schema
must not be used against a migrated store. Preserve old records and provide an implementation-level
backup/rollback procedure; do not downgrade or rewrite authenticated bytes.

The first coherent source implementation is a Harness owner change adding durable draft/item,
revision, preview, and mutation-receipt records plus an atomic store migration and the typed owner and
HTTP operations. It reuses the existing owner, binding, source, render, and receipt seams and keeps
control receipt v2 unchanged. Focused owner tests cover eligible-item resolution against actual
stored bytes; metadata versus content grants; include/exclude and protected prerequisites;
pin/unpin; attributed notes and bounds; objective denial without its separate grant; draft CAS and
base-revision drift; preview from the production render path; objective success with a separate
owner-verified grant; exact idempotent recovery; and generic same-ID/different-payload conflict.
Existing source and receipt records must survive migration byte-for-byte and decrypt with their
prior AAD.

The next Console change adds trusted-subject grant issuance at a real authentication ingress and the
production owner adapter. Keep the protected owner credential inside protected configuration; never
put it in request bodies, URLs, owner records, browser storage, or logs. Do not advertise operations
that the actual Harness owner cannot serve. Populate the sidecar only after the independent ADR 0021
join succeeds.

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

- Harness source baseline: `982414ea9f2383bc92806660d30e946a0d8030d1`; see
  `context_owner_support.rs`, `context_owner_source.rs`, `http_routes_run.rs`,
  `context_control/types.rs`, `context_control/store_schema.rs`, and
  `production_context_owner/source.rs`.
- Context Console source baseline: `cde74812cfbe93faf47c6cb3b390fc5a4c807769`; see
  `harness_facade.rs`, `docs/decisions/0021-owner-capability-sidecar.md`, and its authenticated
  permission/owner-port contract.
- Scope: [Harness #391](https://github.com/AI-Ascension/sts2-harness/issues/391),
  [Context Console #18](https://github.com/AI-Ascension/ascension-context-console/issues/18), and
  [Harness #795 option C](https://github.com/AI-Ascension/sts2-harness/issues/795).
