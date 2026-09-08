# Worker ledger safety

The worker ledger separates durable observation from execution authority. A successful handoff
admission returns a tagged `WorkerAdmissionOutcome`:

- `Acquired` contains the retained handoff and an opaque, one-time
  `WorkerExecutionPermit`.
- `Duplicate` contains only the retained handoff and never carries a permit.

`WorkerExecutionPermit` has private fields and no `Clone`, serialization, or public constructor.
It is bound to the in-memory incarnation of the `ExecutionStore` by a private `Weak` token. The
`mark_worker_handoff_running` operation consumes the permit and checks that token, the current
control authority, the immutable handoff tuple, and the `admitted` state in one transaction. A
duplicate, an unknown reservation, a reopened store, or another store instance cannot manufacture
or reuse execution authority. Dropping a permit without starting leaves the durable reservation
for reconciliation.

The worker control row retains the currently authorized watchdog boot while a replacement worker
is stopped and unauthenticated. Reauthorization still requires the current worker boot, the
authenticated owner proof, and a strictly higher mode sequence. An old worker boot or a previously
seen watchdog boot is rejected; restarting a worker does not silently release or restart retained
handoffs.

Worker-handoff reads use a bounded SQL projection for `terminal_record`. SQLite type and length
sentinels are selected alongside a body that is materialized only when it is a BLOB no larger than
the 16 KiB terminal limit. Oversized BLOBs, text values, mismatched terminal columns, and malformed
canonical records fail as corruption. A legacy nonterminal `NULL` remains readable.

Terminal references use the worker wire contract's exact UTF-8 byte bound of 1..1024 bytes and
reject control bytes (including DEL). A terminal is round-tripped through the canonical record
before its acknowledgment digest is used. The owner `CompletionRecord` applies this same bound
only to its `terminal_ref` field so a worker completion remains readable after restart; other
execution references and provider result fields retain the core 512-byte reference bound. Worker
failures update the durable job to `failed`, while retaining the terminal receipt and preventing
an implicit replacement claim.

These rules are exercised by the deterministic execution-store tests. They do not prove a live
worker process, transport, provider, gateway, host, or release integration.
