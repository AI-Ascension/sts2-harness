# ADR-WF-004: Transactional workflow state

Status: accepted for the Phase 1 durability foundation. Owner: harness maintainers.

The workflow store is a harness-owned extension of the admitted execution-store candidate. It
resides in the execution/store boundary and is exposed through typed methods on `ExecutionStore`;
workflow runtime orchestration remains a later consumer. The foundation does not add a scheduler,
transport, CLI/API surface, gateway authority, MCP framing, or game behavior.

Workflow definitions, plans, runs, episodes, commands, game operations, invocations, and events use
separate Rust newtypes and separate SQLite columns/tables. Definitions and plans retain their exact
bytes and SHA-256 digest. Run events carry a unique event identity, a per-run sequence, normalized
payload, and payload digest. `workflow_events(run_id, sequence)` and the run revision are unique and
monotonic. Run projections retain status, cursor, stack, and bounded counters; replay starts from
the stored initial projection and rejects gaps, digest changes, invalid transitions, or projection
mismatch.

Every event append uses `BEGIN IMMEDIATE` and compares the caller's expected run revision before
inserting the next sequence. Invocation intent reservation, its `reserved` event, and the run
revision advance commit together. A send marker and completion are separate short transactions
around external work. The APIs do not accept an external callback while a SQLite transaction is
open. A committed reservation without a send marker remains unresolved and is eligible for later
reconciliation; absence of a local marker never proves that an external effect did not occur.

The existing execution schema remains versioned by SQLite `PRAGMA user_version`; the workflow schema
is independently pinned in store metadata as `workflow_schema_version = 1` and
`workflow_contract = workflow-v1`. Unknown or newer SQLite versions, missing metadata, malformed
payloads, failed integrity checks, and SQLite `FULL` errors fail closed. Opening read-only state
does not create or migrate it. The store requires WAL, `synchronous = FULL`, foreign keys, a bounded
busy timeout, and a local file deployment. The bundled binding is `rusqlite 0.40.2` with
`libsqlite3-sys 0.38.2`, whose bundled library reports SQLite 3.51.1; durability remains
conditional on filesystem, operating-system, and hardware guarantees.

The admitted owner-local candidate is the pre-worker execution-store commit `5d9806b`. The later
worker-ledger branch was not adopted. Focused process tests cover identity immutability, migration
and settings, restart/replay, compare-and-swap concurrency, intent/send/completion identity, the
external-I/O boundary, corruption, and newer-schema refusal. They are deterministic local-store
evidence and do not establish native game, provider, gateway, MCP, or deployment compatibility.
