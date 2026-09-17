# ADR 0044: Served managed-context source adoption and rendering

## Status

Accepted for the Harness #95 producer implementation. The served acceptance fixture covers the
production `serve-workflow` process against pinned Gateway/MCP peers and a local offline provider
bridge. It does not claim native provider, game, deployment, or Studio UI acceptance.

## Context

The selected context limits and renderer existed before a production invocation used them. A live
run needed a trusted source of retained context content, an explicit way to publish and activate
that content, and a worker path that used the exact current run binding and selected limits before
provider dispatch. An empty registry or caller-supplied matching digest would not establish source
authority.

The served session launches and observes the runtime when a run is submitted. The decision node is
reached by a later workflow `Step`, so an operator can publish and adopt a source after submission
and before the first decision invocation without pausing a provider call or fabricating an owner
binding.

## Decision

The production context owner accepts an explicit `render_required` configuration and a bounded
allowlist of immutable source identities. Configuration authorizes which source identities may be
published; it does not publish or activate them. A source document contains a context draft and its
referenced items. The owner checks the exact digest against its allowlist and stores the document
encrypted in the existing per-run ContextControlStore.

The authenticated workflow routes are:

```text
GET  /v1/workflow-runs/{run_id}/context-owner-source-status
PUT  /v1/workflow-runs/{run_id}/context-sources/{source_id}
POST /v1/workflow-runs/{run_id}/context-sources/{source_id}/adopt
POST /v1/context-bindings/bind
```

Status reads require `workflow:read`; publication requires `workflow:content:write`; binding and
adoption require `workflow:control`. Upload responses expose identity metadata, not context bytes.
Adoption uses the current control version, revision and boundary as compare-and-swap preconditions.
The owner derives preview and approval digests from the encrypted source document, then commits the
active source reference and control receipt atomically. Repeated adoption is idempotent through its
idempotency key and receipt.

Adoption requires a current owner binding for the exact persisted run cursor, definition digest,
trusted runtime instance, decision node and node execution ID. It refuses when no current binding
exists; it does not fabricate a binding from configuration or request fields.

At each actual decision, the production session resolves the current owner observation, legal-action
catalog, context binding, owner lease, active source revision and selected limits. It composes the
descriptor’s effective render limits, prepares immutable bytes for that exact `DecisionInput`, and
sends those bytes through `ExoSession::decide_prepared`. It rechecks the source and runtime fences
after the provider returns and discards a result if the active source or boundary changed while the
request was in flight. The owner lock is not held across provider I/O. Managed decisions invalidate
any retained plan tail before preparing a new request, so an earlier response cannot stand in for
newly rendered bytes.

The served setup sequence is:

1. Submit the admitted live workflow. The service allocates the runtime and publishes its first
   observation and catalog to the owner.
2. Read source status and publish the exact allowlisted source document.
3. Step the workflow through its observation node and read the new persisted decision cursor.
4. Bind that exact cursor, adopt the published source using the returned owner boundary, then Step
   the decision node.

If a decision reaches a render-required owner with no active source, the owner refuses before the
provider port is called. The workflow marks that execution failed and cleans up the live session;
the same run cannot be retried after that terminal failure. Publish and adopt before the decision
Step, or submit a fresh run if a missing-source Step already failed.

## Compatibility

The metadata-only owner configuration remains available and does not claim managed content. The
render-required configuration is additive and fails closed when its source is missing, stale,
expired, unbound, outside the current run boundary, or beyond a selected limit. Existing workflow
and context-control receipt schemas remain unchanged; the source upload and adoption messages are
versioned additions.

## Consequences and limits

- The selected `max_items`, `max_notes`, `max_context_bytes` and `max_objective_bytes` limits reach
  the actual served preparation call. An overage refuses before provider exchange.
- An explicit source adoption activates one immutable source revision. Later decisions render from
  that revision automatically; there is no per-inference operator approval.
- Provider inference is synchronous and cannot be cancelled after admission. If the source or
  runtime boundary changes while it is in flight, the result is discarded. A later decision
  resolves the currently active source again against the live binding and selected limits.
- The process fixture uses exact pinned Gateway/MCP revisions and a local deterministic bridge. It
  proves served routing and prepared request bytes, not native provider billing, game effects,
  gameplay correctness, or release compatibility.
