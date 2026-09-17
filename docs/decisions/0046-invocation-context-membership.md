# ADR 0046: Invocation-Scoped Context Membership

## Status

Accepted for the Harness context-control boundary. This record does not authorize
a provider, native-host, game, deployment, or paid-call lane, and it does not
change the host's authority over legality, mutation, or observations.

## Decision

The Harness adds a versioned, per-invocation membership policy. Until this record,
one owner-issued `ContextDraft` decided what entered every model request: two
invocations could not disagree about inclusion without rewriting a persisted
revision, nothing recorded *why* an item was included or excluded, and no value
could retain a mandatory host prerequisite in owner state while omitting it from
model-visible input.

`ContextMembershipPolicy` is the per-invocation input. It carries one disposition
— `include`, `exclude`, or `inherit` — over the draft selection, its own item
overrides, an optional pin inheritance, an optional wider-scope capability, and an
explicit model view. Resolving it produces an `EffectiveMembership` that names
every included and excluded reference with a typed reason, separates the retained
`mandatory` prerequisites from the `model_visible` subset, and binds the whole
decision with a policy digest.

Compatibility: the change is additive and in-process. No existing field, route,
durable record, or published schema changes. The policy schema identity is
`ascension.context-control.membership.v1`; it has no published consumer and no
HTTP projection, so no contract document is added for it. A later incompatibility
requires a new versioned schema, not an in-place field change.

## Scope is carried by item kind

Item identity carries no per-invocation marker, so scope is expressed by
`ContextItem::kind`. The shared kinds (`history`, `strategy`, `objective`) are
episode-independent and carryable; any other kind is owned by one invocation's
agent/episode and is default-deny. Crossing that boundary requires a wider-scope
capability that names both the calling agent and the individual item, so an
authorization for one item cannot carry a sibling's item by accident. A refused
crossing is distinguishable: no authorization at all is a sibling-scope leak,
while an authorization that does not cover the specific item is narrower than the
request.

## Owner prerequisites and the model view

A protected item is retained as a mandatory prerequisite for owner legality, host
state, and programmatic routing, and is suppressed from model-visible input
rather than dropped. Collection and inclusion stay independent, and a policy may
not exclude a protected prerequisite at all.

## Effective absence needs executable continuity *and* an omission wireform

A membership policy selects which already-collected items become model-visible
context. It cannot erase what an opaque persistent provider adapter already
received, so effective absence was never executable for a binding that keeps
provider-side history. The stateless case cannot substantiate it either: the
managed render path composes the provider request from an input that always
carries the observation, so an admitted `observation_visible: false` would still
publish the observation while reporting it hidden. That is a fail-open in the
prepared bytes, of the same class this boundary exists to prevent.

Effective absence of an observation is therefore **refused for every
continuity** until an omission wireform exists that the render path actually
consumes. Refusing at preparation keeps the gate's verdict identical to what the
provider receives; the refusal reuses `EffectiveAbsenceUnsupported`, so callers
keep the same precise pre-dispatch reason code. Implementing the omission is the
successor work item: it must remove the observation from the composed provider
request while preserving the host-owned observation and legal catalog for owner
legality and routing, and it must ship with a regression proving the observation
value is absent from served bytes.

## Revalidation and historical evidence

An unpin or an exclusion changes only the **next** prepared input. The already
prepared set and its digest stay reproducible: `EffectiveMembership::revalidate`
re-derives the set from the current registry and refuses anything that moved,
including a policy that no longer matches the digest that prepared the set.
Revocation, expiry, digest mismatch, and the mandatory-plus-pin and effective
item bounds are enforced at preparation, before dispatch.

## The boundary is consumed by the production render path

A resolved policy only changes behaviour if the live render consults it. The
production seam is `render_with_membership`, reached by both the live managed
dispatch path and the composed owner render path (`prepare_managed_render`). It
resolves and gates the effective set with `prevalidate_and_bind` **before** any
provider bytes exist, projects `effective.model_visible` onto a clone of the
draft, narrows the draft's pins to model-visible ids so a pin can never name a
prerequisite the model cannot see, and only then delegates to
`ContextRenderer::enabled_at_with_limits`. An effective set the selected owner
limits cannot accept is still refused with `ExceedsSelectedLimit`, because the
owner bound composes with the membership bound rather than being replaced by it.

The owner contributes `ContextMembershipSelector` — disposition, overrides, pin
inheritance, wider scope, and model view, all independent of the invocation —
and `bind` mints the finished policy against the invocation identity and the
draft's `base_revision_id`. The identity is therefore never supplied by a
selector, and a selector cannot carry a default or override belonging to another
invocation. An invocation with no selector in force renders exactly the bytes it
rendered before this boundary existed.

`MembershipContinuity` is derived from the selected binding's
`provider_session_continuity`, not asserted by a caller: a binding that keeps
provider-side history yields `OpaquePersistent`, and one that does not yields
`Stateless`. A caller therefore cannot claim executable absence for either
continuity: the managed render path has no omission wireform, so it would report
`observation_visible == false` while still shipping the observation.

`ContextMembershipScope::branch_id` is optional. The live render seam is driven
by an admitted run, episode, and agent; durable branch continuation is selected
by a separate runtime entry point and is never projected onto the render source.
Absence is encoded as `None` rather than a fabricated id, so branch scope is
enforced honestly rather than satisfied by a placeholder. Real per-branch
isolation remains its own work item (harness #118).

## Evidence

The decision is exercised by deterministic synthetic fixtures only; no provider,
host, or game is contacted. Provider/native acceptance for the wider feature
remains a separate, unverified gate.
