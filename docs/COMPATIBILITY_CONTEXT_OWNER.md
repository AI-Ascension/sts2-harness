# Compatibility: context-owner management surface

This file holds the compatibility rows for the context-owner side of the management surface
(bindings, associations, composed limits, receipts and control commands). It is part of the
[compatibility policy and matrix](COMPATIBILITY.md); the classification vocabulary, evidence
rules and the remaining rows live there. Every shipped row below is `additive-compatible` unless it
says otherwise. Proposed rows are explicitly labeled and do not establish native, provider or
deployment behaviour.

## Owner-local drafts, previews and eligible items; Console sidecar remains unavailable

[ADR 0080](decisions/0080-context-owner-draft-preview-and-sidecar-contract.md) records the
coordinating owner's T1 contract. Harness now implements an owner-local served eligible-item,
draft, revision, metadata-preview and exact mutation-receipt surface backed by its run-scoped
encrypted SQLite owner state. Create, patch, preview and receipt lookup are served through the
existing management owner path; eligibility can be scoped to one authenticated actor's exact
`draft_id`. A request without that selector keeps the immutable-catalog-only projection. Preview
returns bounded metadata and digests, never prepared provider bytes, makes no provider call, and
does not adopt the edited draft. These route and storage additions are additive; Harness #391 and
Context Console #18 remain open for separate-process and consumer acceptance.

The immutable `ContextSourceDocument` remains a published source snapshot, not a mutable draft.
The owner resolves exact references and digests from its own run-scoped encrypted source/item
records and current binding. Caller-supplied IDs or digests cannot create eligibility. Include and
exclude preserve membership policy and protected-prerequisite rules; pins remain a subset of
effective inclusion. Notes and objectives are item references backed by bounded encrypted owner
bytes, attributed to the authenticated same actor. Creating text requires a live finite retention
horizon inherited from the exact active advertised source, rechecked on mutation, preview, and
content projection; text is refused when that source or finite horizon is unavailable. Draft
content bytes require both read and `workflow:context:content:read`; metadata projection reveals no
bytes. Draft creation, note edits, and metadata-only preview require the independent
`workflow:context:edit` grant, while objective edits additionally require
`workflow:context:objective:edit`. These grants do not imply control, metadata read, or content
read.

The v1-to-v2 migration is additive and authenticates the requested existing run using its own key
inside the schema migration transaction. A nonempty v1 database must first be opened with the
valid key for an existing run before a new run can be created. Other run rows and ciphertexts are
preserved byte-for-byte but are not attested by that migration; each is authenticated when opened
with its own key. Empty and metadata-only v1 databases may initialize.

This slice does not publish an edited draft as a new immutable source or make it adoptable by the
runtime. Existing reference-only commit/adoption behavior is unchanged; a new authored digest stays
ineligible for that path. A follow-on owner-local publication operation needs to issue a fresh
immutable identity from the accepted draft, bind it to the current run and complete owner binding,
require explicit publish and adopt authorization, then persist the immutable source, draft/history,
and exact receipt atomically with CAS and retention limits. Restart and the captured catalog must
fence stale publications. No caller-provided digest may mint the identity. Until that operation is
implemented and reviewed, edited drafts are not active context.

Console's existing facade grants and public schemas remain distinct and unchanged. Its proposed
owner adapter may add a sidecar only when the exact accepted capability operation joins the actual
owner record to independently trusted descriptor, owner/revision, source, complete scope, binding
and epoch values. The provider-session effective-limits route is candidate metadata, not that join;
the context-memory record and live provider-session inspection remain unavailable in the current
composition. The internal Console sidecar is therefore absent. The Harness route implementation
does not establish Console integration, a native-game run, or provider-session continuity.

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
