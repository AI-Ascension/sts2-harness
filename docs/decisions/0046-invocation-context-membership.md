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

## Effective absence needs executable continuity

A membership policy selects which already-collected items become model-visible
context. It cannot erase what an opaque persistent provider adapter already
received. Effective absence of an observation is therefore refused unless the
continuity in force is verified stateless, fresh, or reconstructed. The
alternate would claim a selector change erased provider history, which the
Harness cannot substantiate.

## Revalidation and historical evidence

An unpin or an exclusion changes only the **next** prepared input. The already
prepared set and its digest stay reproducible: `EffectiveMembership::revalidate`
re-derives the set from the current registry and refuses anything that moved,
including a policy that no longer matches the digest that prepared the set.
Revocation, expiry, digest mismatch, and the mandatory-plus-pin and effective
item bounds are enforced at preparation, before dispatch.

## Evidence

The decision is exercised by deterministic synthetic fixtures only; no provider,
host, or game is contacted. Provider/native acceptance for the wider feature
remains a separate, unverified gate.
