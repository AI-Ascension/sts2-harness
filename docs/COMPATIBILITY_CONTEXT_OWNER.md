# Compatibility: context-owner management surface

This file holds the compatibility rows for the context-owner side of the management surface
(bindings, associations, composed limits, receipts and control commands). It is part of the
[compatibility policy and matrix](COMPATIBILITY.md); the classification vocabulary, evidence
rules and the remaining rows live there. Every shipped row below is `additive-compatible` unless it
says otherwise. Proposed rows are explicitly labeled and do not establish native, provider or
deployment behaviour.

## Proposed owner-local drafts, previews and capability sidecar

[ADR 0080](decisions/0080-context-owner-draft-preview-and-sidecar-contract.md) records the
coordinating owner's T1 contract for a future Harness owner-local eligible-item, draft, revision,
preview and mutation-receipt surface, plus Console's internal ADR 0021 capability-sidecar trust
join. This PR is documentation only: the routes, durable records, new scopes, Console adapter and
separate-process acceptance do not yet exist. Harness #391 and Context Console #18 remain open.

The current immutable `ContextSourceDocument` remains a published source snapshot, not a mutable
Console draft. Its item bytes are the candidate owner-authoritative source for future eligibility;
today there is no public eligible-item operation or durable mutable-draft registry. A new owner
operation must resolve exact references and digests from its own run-scoped encrypted source/item
records and current binding. Caller-supplied IDs or digests cannot create eligibility. Include and
exclude must preserve membership policy and protected-prerequisite rules; pins must remain a subset
of effective inclusion. Notes and objectives remain item references backed by owner-held bytes.
New note/objective bytes must stay in bounded encrypted owner records with existing expiry and
retention; author identity comes from the authenticated same actor.

The proposed permission additions are `workflow:context:content:read` for owner-advertised bounded
eligible item bytes, `workflow:context:edit` for draft, membership, note and preview mutations, and
`workflow:context:objective:edit` for objective changes. Metadata reads and exact receipt recovery
remain `workflow:read`; pause, commit and resume remain `workflow:control`; immutable source upload
remains `workflow:content:write`. The new content scope reveals only bounded eligible items under
existing protection, expiry and retention policy; it does not expose prepared provider input, enable
capture, or authorize caller-selected references. Preserve the existing `workflow:*` wildcard
behavior and same authenticated-subject binding. A narrow ordinary edit or control grant does not
imply objective edit, and a metadata-only caller receives no raw item bytes. New closed owner
envelopes and an atomic one-way SQLite v1-to-v2 migration preserve all old source/receipt bytes and
AAD; new records use purpose-specific AAD and do not extend retention.

Console's existing facade grants and public schemas remain distinct and unchanged. Its proposed
owner adapter may add a sidecar only when the exact accepted capability operation joins the actual
owner record to independently trusted descriptor, owner/revision, source, complete scope, binding
and epoch values. The provider-session effective-limits route is candidate metadata, not that join;
the context-memory record and live provider-session inspection remain unavailable in the current
composition. Until a real adapter proves the trust join, the internal sidecar remains absent. The
versioned scopes/envelopes are proposed additions; they are not yet compatibility claims for a
shipped server.

## Opt-in recorded context bindings

[ADR 0040](decisions/0040-recorded-context-binding-history.md) adds private, bounded SQLite
binding history and a scoped library-only historical reader. Public closed JSON schemas,
current-cursor association semantics and file-store JSON remain unchanged. Old SQLite stores
start with no history; rollback binaries ignore and preserve the added table. Retention is
explicitly enabled and does not imply current owner availability or control permission.
The unreleased Rust `CommandApplication` gains `context_binding`; source constructors must
set `None` or provide exact accepted binding evidence. Default `WorkflowStore` hooks remain
unsupported, and existing default compositions do not retain this metadata.

## Composed context-owner effective limits

[ADR 0030](decisions/0030-context-owner-effective-limits-composition.md) adds
`GET /v1/workflow-runs/{run_id}/context-owner-effective-limits`, returning
`ascension.harness.context-owner-effective-limits-view.v1`: the limits advertised by the catalog
descriptor that admits the owner's **current** binding for one run, composed through the same
fail-closed seam (`compose_context_owner_binding`) that live admission uses. This is
`additive-compatible`: `GET /v1/context-bindings`, `POST /v1/context-bindings/bind` and the ADR 0025
association projection are unchanged, no bound, schema, digest or default changes, and the admission
refactor keeps the existing error codes for the checks it moves. A foreign owner, a missing or
disabled descriptor, a non-available binding, a non-published binding identity, a grant or
continuity escalation, an oversized descriptor and a stale descriptor/catalog digest are refused
before any advertised value is reported; an unattached owner stays explicitly unavailable. The
projection itself is observation-only; the enforcement of the selected limits is wired separately
by [ADR 0041](decisions/0041-selected-limit-enforcement-wiring.md).

## Selected context-control limit enforcement

[ADR 0028](decisions/0028-selected-context-control-limit-enforcement.md) adds
`ContextRenderLimits` and `ContextRenderer::enabled_at_with_limits`, which enforce the limits a
binding actually advertises (`max_items`, `max_notes`, `max_objective_bytes`, `max_context_bytes`)
rather than only the harness maxima, reporting `ExceedsSelectedLimit` with the offending limit name.
This is `additive-compatible`: `enabled`, `enabled_at` and `legacy` are unchanged and the harness
maxima are untouched. [ADR 0041](decisions/0041-selected-limit-enforcement-wiring.md) wires the
render entry point and `max_control_events` to their production points of use, so the selected
limits are enforced rather than only validated.
Applying a selected event bound also rejects an authority whose retained journal already exceeds
that bound with `context_control_events_exhausted`, matching bounded recovery. A journal exactly
at the selected bound remains admissible; existing events are never silently discarded.

[ADR 0058](decisions/0058-served-whole-input-output-reserve.md) adds one optional
`output_reserve_bytes` to `ContextRenderLimits` and to the descriptor's `ContextEffectiveLimits`,
and admits the assembled provider bytes of a served managed decision against it before any dispatch.
This is `additive-compatible`: absent is exactly the prior contract, the field is skip-serialized so
existing descriptors and the committed conformance fixture are byte-identical, no route, digest or
default changes, and no capacity is raised. It does not establish that the input-plus-response
composition was bounded before a reserve was advertised.

## Current context-owner association

[ADR 0025](decisions/0025-context-owner-current-association.md) adds
`GET /v1/workflow-runs/{run_id}/context-owner-association`, returning the owner's current
`ContextOwnerBinding` as `ascension.harness.context-owner-association-view.v1`. This is
`additive-compatible`: the existing `GET /v1/workflow-runs/{run_id}/context` `ContextAssociation`
contract is unchanged, no record/schema/digest or bound changes, and unknown paths still fail closed.
The projection is observation-only; projected grants and epochs are owner assertions and confer no
harness-issued control authority.

## Recovered context-control receipts

[ADR 0024](decisions/0024-context-control-receipt-recovery.md) adds
`POST /v1/workflow-runs/{run_id}/context-control-receipts/lookup`, which returns the owner's already
recorded `ascension.context-control.owner-receipt.v2` for a retained `pause`/`commit`/`resume`
command. This is `additive-compatible`: the port method has a failing default, no existing owner,
binding, receipt or digest changes, and nothing is re-issued, re-applied or inferred. Recovery
requires scoped read authorization and an owner that can return persisted historical evidence; the
receipt must match the exact owner/invocation/binding/command identity. It does not require or
recreate a current association, and it grants no new control authority. Unsupported, unrecorded,
mismatched and unavailable outcomes stay distinct.

## Context-owner control commands

[ADR 0048](decisions/0048-context-owner-control-commands.md) adds
`POST /v1/workflow-runs/{run_id}/context-control-commands` (`workflow:control`), submitting one
`pause`/`commit`/`resume` `ContextControlCommand` to the authoritative owner for the run's current
binding and returning its v2 receipt; the optional `STS2_WORKFLOW_TOKEN_<PROFILE>_READ` companion
mints a `workflow:read`-only token for the same profile subject. This is `additive-compatible`: no
existing route, record, schema, digest or default changes, and the harness mints no authority. An
exact duplicate returns the recorded receipt without a second effect, and a stale control version,
boundary or revision fence is refused (409, no receipt) before the owner is called. Evidence is
synthetic and in-process only.

## Recorded context-binding HTTP projection

[ADR 0023](decisions/0023-recorded-context-binding-http-projection.md) adds one read-only
management route (`GET /v1/workflow-runs/{run_id}/executions/{node_execution_id}/context-binding`)
returning the versioned `ascension.harness.recorded-context-binding-view.v1` projection of the
binding accepted for that invocation. This is `additive-compatible`: no existing route, record,
schema, digest or resource bound changes, retention remains opt-in, and unknown paths still fail
closed. The projection is observation-only and is neither current owner authority nor receipt
recovery.
