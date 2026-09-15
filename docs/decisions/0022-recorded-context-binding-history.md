# ADR 0022: Recorded context binding history

## Status

Proposed for independent review. Scoped implementation authorized for Harness #100;
no production owner, consumer HTTP contract or feature closure is approved here.

## Decision and ownership

The harness management store owns immutable historical evidence of the context-owner response
accepted for a workflow invocation. `ManagementService::with_context_binding_history` explicitly
opts into retaining this metadata with command results. SQLite supports it; file and memory
stores reject enabling it. Default service compositions do not retain new metadata.

The live executor returns only the binding accepted by the current command. Later steps, pause
and cancellation cannot republish a prior binding. Before persistence, the binding must match the
accepted pre-command run, definition and graph/node/node-execution identity. Its owner ID/version,
descriptor identity/digest and narrowed grants were checked at dispatch. Structural agreement is
not proof of a real owner, fresh epoch or game/provider execution.

The private `ascension.harness.context-binding-history.v1` record preserves the binding, original
authenticated subject, command ID and applied run revision. It uses a dedicated SQLite table,
foreign-keyed to the command, unique by both invocation and command. History, command result,
snapshot and applied event commit in one transaction. Existing command/request digests continue
to govern replay; a changed history supplied on store-level replay conflicts.

## Historical read and authorization

`recorded_context_binding(actor, run_id, node_execution_id)` is a library-only historical read.
It requires enabled retention, the caller's current scoped `workflow:read` permission and the
same subject as the original binding command. Removing the read scope or changing the run prefix
denies access even to that subject. No owner lookup, rebind, provider call or continuation occurs.

Returned grants, epochs and continuity flags are facts of the original response, never current
capabilities. A revoked or unavailable owner may still have historical evidence; that evidence
cannot authorize execution or control. Current control continues through the separately scoped
owner port. This slice does not add grant-revocation infrastructure to `AuthContext`.

The existing v1 context-association endpoint means the **current** cursor and remains unchanged.
The recorded invocation can differ after a decide step advances the runtime cursor. It must not
be mislabeled as current evidence. A future HTTP history projection requires a separately
versioned owner/consumer contract; this library method does not silently supply that projection.

## Bounds, failures and recovery

Each record is at most 16,384 encoded bytes, with at most 256 records per run. New steps check
capacity after command acceptance/replay lookup but before owner or execution calls. At capacity,
even a step that would not bind context is conservatively blocked; pause/cancel remain usable.
The serialized same-run command acceptance prevents another ordinary command from consuming the
checked capacity concurrently. SQLite also checks capacity and record size inside the transaction.
Reads bound record and command-response byte allocation in SQL and revalidate record identities,
schema, run definition and corresponding command result.

The metadata table shares the existing trusted management SQLite storage boundary; it is not
authenticated encryption or protection against an attacker rewriting the entire database.
No content, prompt, provider payload or credentials enter this record. Portable exports omit it.
There is no automatic deletion, resource-ceiling increase, or retention of real private data in CI.

A commit failure rolls back result and history together while leaving the accepted command
in flight. Exact replay reports pending and does not dispatch again. The in-memory runtime may
have advanced; after restart it remains unavailable. This is explicit uncertainty, not a claim
that a provider effect rolled back. A crash after owner acceptance but before command-result
commit can leave no binding history. Pre-effect durable binding intent, authoritative owner
receipt recovery and live session reattachment remain separate work.

## Compatibility and evidence

Public closed JSON schemas, `RunSnapshot`, context-association v1 and exported file-store JSON
are unchanged. The private SQLite table is additive: legacy databases open with empty history,
and old binaries ignore the new table. Rollback cannot read new history but preserves its bytes.
Removing opt-in composition stops new retention; it does not delete existing evidence.

The unreleased Rust `CommandApplication` gains `context_binding`; source adapters constructing
it must set `None` unless they supply the exact accepted invocation binding. `WorkflowStore`
adds default unsupported hooks, preserving existing implementations' default behavior.
This is an explicit source API adjustment, not a claim of unchanged Rust struct-literal
compatibility.

Tests use hand-authored MIT synthetic identities and recording ports. Required validation is
strict repository policy, pinned formatting/Clippy/workspace tests, focused SQLite replay,
authorization, cursor, opt-out, rollback and capacity tests, and unchanged artifact checksums.
No native/provider, production owner composition, HTTP history, Console/Studio journey or
complete Harness #100 acceptance is established.
