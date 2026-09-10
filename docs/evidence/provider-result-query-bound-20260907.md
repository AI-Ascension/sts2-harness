# Provider result query-bound evidence

Date: 2026-09-07. Classification: source and synthetic execution-store evidence.

The five decision reads that can materialize a stored provider result all use the shared
`decision_query` projection in `crates/harness/src/execution/store_provider_queries.rs`:

| Read path | Consumer |
| --- | --- |
| normal decision lookup | `store_provider_queries.rs::decision` |
| completed-result reuse | `store_provider_queries.rs::reuse_completed_decision` |
| idempotent decision recording | `store_provider.rs::record_decision` |
| resume pending-decision enumeration | `store_recovery.rs::pending_decisions` |
| idempotent provider completion | `store_provider_results.rs::finish_provider` |

The projection first preserves SQL `NULL`, then checks `typeof(result_payload) = 'blob'` and
`length(result_payload) <= MAX_DECISION_RESULT_BYTES`. Only the bounded BLOB branch selects the
payload; an oversized BLOB or any non-BLOB value produces a short text sentinel. The row reader
accepts `NULL`, copies only a bounded BLOB, and maps the sentinel or any other SQLite type to
`InvalidQuery`, which the store maps to `ExecutionStoreError::Corrupt`. Thus an oversized or
type-confused value cannot be silently treated as a legacy metadata-only completion.

The focused integration tests exercise all five paths with a 16 MiB `zeroblob` and/or a TEXT
payload, while the existing valid result/reuse test remains passing. The SQL branch bound is
source-derived and test-observed; this campaign does not measure total process memory or claim
that SQLite's pager/cache cannot retain pages for a corrupt database. The conservative claim is
limited to avoiding the oversized value as the selected result expression before
`sqlite3_column_blob`/Rust payload copying.

Validation command and result:

```text
CARGO_TARGET_DIR=<unique-target> cargo test --locked --offline --package sts2-harness --test execution_store
15 passed; 0 failed
```
