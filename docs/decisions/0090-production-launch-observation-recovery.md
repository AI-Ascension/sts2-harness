# ADR 0090: Preserve launch-observation retryability without repeating launch

## Status

Accepted engineering decision for the unreleased production management owner.
Closes the classification decision in [#804](https://github.com/AI-Ascension/sts2-harness/issues/804),
using the submission recovery contract from [ADR 0089](0089-bounded-management-dispatch-recovery.md).

## The launch fence follows writes

The issue's original universal pre-write premise is false. The served submission path
first commits the actor/request claim and durable run reservation. It then opens the
production session and calls `runtime.launch()`. When an enforcing context owner is
attached, it also refreshes the gateway lease binding. Only then does it call
`runtime.observe()` to fence provider construction. No provider session opens before
that observation and the subsequent action catalog/policy gates succeed.

The observation's `PortError::is_retryable()` bit describes the failed read. It cannot
prove the preceding launch, process/session allocation, or durable reservation did not
happen. Retrying the whole submission under a fresh identity would therefore be unsafe.

## Decision

| Launch observation failure | HTTP and envelope | Meaning and required recovery |
| --- | --- | --- |
| Retryable read error | 503, `ascension.management-error/v2`, `write_state_unknown` | The read may be transient; preserve the original request identity and reconcile the retained run before another launch decision. |
| Terminal read/protocol error | 409, `ascension.management/v1`, `conflict` | This launch cannot satisfy its observation fence. The retained reservation still requires recovery; the terminal read failure is not proof of no preceding write. |

Both retain `live_launch_fence_failed` and explicitly instruct the caller to recover the
original `request_id` rather than resubmit under a new identity. Both persist the failed
claim and leave the reserved run in `NeedsOperator`. A repeated POST cannot open another
runtime or provider; authenticated GET recovery returns the original error and run ID.
The terminal owner error remains `unresolved` in that retained recovery result. Its
HTTP refusal keeps the legacy v1 `conflict` projection required by existing clients.

The launch path attempts stop and lease release on either failure. Existing distinct
cleanup and reservation-persistence error codes remain authoritative if those steps
also fail. A failed reservation-status update leaves the original reservation present;
it does not erase the claim or authorize a retry.

The `service_ops` command-release guard is a separate path from RunRequest submission.
Both uncertainty classes retain command identity there under ADR 0089. This change
does not reinterpret that guard as proof that a failed launch made no writes.

Only the first production launch observation uses this mapper. Other runtime seams
keep their existing classification. In particular, `reobserve` and
`coop_receipt_query` retain `RpcReadKind::Recovery`; neither gains gameplay-read
retryability or automatic resubmission.

## Evidence boundary

The regression traverses the actual HTTP server, management service, SQLite claim/run
reservation, live execution port, and `ProductionLiveWorkflowSessionFactory`. The
runtime, target catalog, context owner, and provider ports are explicit synthetic
fixtures. The runtime asserts that its run row exists before launch. Tests observe one
runtime open, one launch, one failed observation, zero provider opens, and preserved
identity across POST replay, GET recovery, and a reopened SQLite store. Separate fault
cases retain cleanup and reservation-persistence diagnostics. The terminal case also
checks the HTTP v1 `conflict` projection separately from the retained `unresolved` error.

Existing wire regressions continue to distinguish gameplay-read retryability from
recovery-read refusal. These tests establish the production composition and wire
classification, not a native game session or paid provider call. They do not reproduce
or resolve Studio's intermittent proxy-level 502.
