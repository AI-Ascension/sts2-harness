# ADR 0042: Authenticated provider-session policy owner API

## Status

Accepted for scoped implementation of Harness #95. The routes provide a durable operator surface;
served live composition and downstream Console/Studio adoption remain separate acceptance work.

## Context

ADRs 0026 and 0027 define portable policy validation, profile admission and explicit migration
approval, but their saved-policy owner was only a library boundary. Operators could neither inspect
the current adopted policy through the workflow API nor import, propose, approve or adopt through an
authenticated command surface. In addition, a valid initial policy has no exceeded limit to migrate,
so it needs its own explicit adoption command.

Policy bytes and approval references are private owner data. The management surface must expose
redacted metadata, scope every command to the workflow run and retain exact upload bytes. A failed
or repeated command must not silently alter history or move the active policy.

## Decision

Expose the owner through the workflow management API:

```text
GET  /v1/workflow-runs/{run_id}/provider-session-policy
POST /v1/workflow-runs/{run_id}/provider-session-policy/import?expected_revision={revision}
POST /v1/workflow-runs/{run_id}/provider-session-policy/proposals/{proposal_id}
     ?source_sha256={digest}&expected_revision={revision}
POST /v1/workflow-runs/{run_id}/provider-session-policy/proposals/{proposal_id}/approve
     ?expected_revision={revision}
POST /v1/workflow-runs/{run_id}/provider-session-policy/proposals/{proposal_id}/adopt
     ?expected_revision={revision}
POST /v1/workflow-runs/{run_id}/provider-session-policy/adoptions
     ?expected_revision={revision}
```

The import and proposal request bodies are the exact policy JSON bytes retained by the encrypted
owner journal. They are not copied into workflow records or returned by the API. Approval and
adoption use strict, closed JSON request records. Every mutation is run-scoped and requires
`workflow:control`; importing a policy or uploading a proposal target additionally requires
`workflow:content:write`. Reads require `workflow:read`.

Mutations use the journal revision as a compare-and-swap precondition. Repeating the same import
digest, proposal identity and target digest, approval reference, or adoption while that exact result
is still current is idempotent and does not advance the revision. Reusing an identity with different
content, a stale revision for a new change, or an invalid target fails without changing the journal.
Adoption remains explicit: an initial imported policy uses `/adoptions`, while a migration proposal
must first be separately approved and then adopted with that same approval reference.

The `ascension.provider-session.policy-owner-view.v1` GET projection exposes the active policy's
bounded execution metadata, redacted policy history, proposal state and owner revision. It omits
policy bytes, credential realm references and approval values. It includes proposal digests so an
operator can continue approval/adoption after refreshing history or recovering a lost response.
Command responses report metadata only and declare zero inference calls and zero game effects.

The journal remains bounded and encrypted by `ProviderSessionMetadataStore`. Startup distinguishes
a genuinely absent journal from I/O, authentication or corruption failures, then verifies every
retained policy's exact-byte digest and scope, the active policy's profile admission, and every
proposal's source bytes, target, capability binding and immutable proposal digest before serving
metadata or commands.

## Compatibility

Additive management routes and typed request/response records. Existing provider-session and
workflow contracts are unchanged. No raw policy bytes, credentials, approvals, prompts or provider
outputs are added to workflow events, logs or responses. Native calls and game effects are outside
these routes.

## Consequences and limits

- The API allows an authorized operator to retain, review and explicitly activate an initial or
  migrated policy without clamping policy values.
- A served live workflow must attach the same trusted policy owner to its pre-provider admission
  boundary. The HTTP route alone does not establish that the executable runtime loads the adopted
  revision.
- Provider execution, native compatibility and Console/Studio journeys remain unverified until
  their respective production callers and acceptance evidence are complete.
