# ADR 0089: Bound management dispatch waits and retain submission identity

## Status

Accepted engineering decision for the unreleased owner-local management surface.
Refs [#818](https://github.com/AI-Ascension/sts2-harness/issues/818).

## Problem

The HTTP adapter bounded request reads and response writes separately, but invoked
synchronous dispatch between those phases with no deadline. A stalled definition,
capability, store or execution port could hold its connection indefinitely. Ending a
wait does not establish that the operation performed no writes. A caller that chooses
a new submission ID after that response can duplicate a launch.

## Decision

Each server owns a finite dispatch executor bounded by `HttpLimits.max_connections`.
Dispatch receives an independent `dispatch_deadline`, default 30 seconds and validated
in the interval greater than zero through 300 seconds. The 30-second default is an
operational budget, not a measured native/provider latency guarantee. The existing
five-second read and write budgets each begin at their own phase. The shipped client
keeps five-second connect/write limits and waits at most 45 seconds overall, covering
all default server phases plus a five-second scheduling margin.

Route effects, rather than the HTTP method, determine an expired or disconnected
dispatch response. Exact read routes return HTTP 503 `unavailable`. These include
definition validation, inspection and diff, target preflight, offline replay, redacted
export and historical receipt lookup even though they use POST. Unknown route shapes
remain conservatively classified as possibly mutating until explicitly reviewed.
A possibly mutating request returns HTTP 503 with `write_state_unknown` under
`ascension.management-error/v2`. Existing
errors retain `ascension.management/v1` and its status-derived HTTP classes: owner
capability, unresolved and budget errors remain HTTP 409 `conflict`; HTTP 404 remains
`invalid_input`. The explicit `store` class is preserved at HTTP 503 so a persistence
failure remains distinguishable from general unavailability. Retained submission
errors keep their original owner class. These mappings avoid extending the closed v1
enum discussed in ADR 0047. An old strict client must reject the unknown
version and preserve the original operation identity. It must not translate an unknown
error envelope or broken connection into permission to resubmit with a new ID.

A timed-out synchronous worker keeps its executor slot and all captured resources until
it finishes. No new handler starts when capacity is exhausted. When capacity exceeds
one, one slot is reserved for authenticated submission-recovery GETs; capacity one has
no such additional slot. Only the exact, query-free recovery route with a valid request
identifier can use that slot. Completed workers are joined and removed.

The server thread supervises both connection and dispatch workers. On shutdown or an
admission failure it closes the listener, then keeps every worker handle until it has
joined all accepted work. `ServerHandle::shutdown(&self)` stops admission and waits up
to 100 milliseconds for that supervisor, returning `dispatch_shutdown_pending` if work
remains. Its listener wake has a separate maximum of 50 milliseconds. Shutdown no longer
consumes the public handle: the caller may inspect `pending_dispatches()`, repeat shutdown,
or explicitly `wait()` until all handlers are joined. The latter can wait indefinitely
for an uncooperative port and is not a bounded cancellation operation.

A process-local registry retains each supervisor before returning its public handle.
Dropping a handle therefore stops admission without dropping unfinished worker ownership.
The registry admits at most 128 active or draining servers; pending owners are never
evicted to admit replacements. A full registry refuses server creation before spawning
a supervisor. Subsequent server starts, completed shutdown/wait calls, or an explicit
`ManagementServer::reap_retired_servers()` join completed supervisors and release their
registry entries. A retained owner whose state cannot be read remains capacity-consuming.
An abandoned handler can occupy this finite capacity indefinitely; this is a visible
availability limit, not a cancellation claim.

The budgets apply to their read, dispatch-wait and write phases. Connection lifecycle
logging and undeliverable-response reporting still call synchronous sinks outside those
waits. A blocked custom sink or backpressured production stderr can therefore hold a
connection before dispatch or during teardown. Its connection handle remains supervised,
but bounding that diagnostic I/O is also unfinished #818 work; this component does not
claim an unconditional end-to-end connection-termination deadline.

Arbitrary synchronous Rust ports cannot be forcibly cancelled safely. Neither a response
deadline, retained supervision nor a pending shutdown response establishes that the
stalled handler has terminated. The termination acceptance criterion of #818 remains
unfinished internal engineering work. It requires deadline-aware production ports and
bounded lock/I/O operations, or an owned process boundary for non-cooperative components;
merely moving a blocking call to another detached thread is insufficient. Terminating an
isolated worker would still require durable write-identity reconciliation, not a claim
that an external write was undone. Process termination requires that reconciliation on
restart as well.

## Submission fence and recovery

V1 submission commits an actor-subject digest and canonical request digest before its
first external port call. Memory and file adapters arbitrate within one store instance;
the file adapter requires a single owner and does not coordinate independent writers.
SQLite uses an immediate transaction across separate connections. One request identity
can only acquire one claim. A repeated matching actor/payload reads the retained result;
a different actor or digest conflicts. A legacy run reservation with no authenticated
claim is refused rather than adopted by guessing its request ID.

Claims have no expiry or implicit reset. There are at most 4096 per store, each at most
16 KiB of serialized metadata. Exhaustion
refuses further new claims, while existing claims remain recoverable. There is no new
purge API: deleting unresolved claims would erase the duplicate-launch fence. Memory
storage is process-local; file and SQLite claims survive restart. Production recovery
requires a durable store.

`GET /v1/workflow-submissions/{request_id}` requires `workflow:read` and the original
authenticated subject. Foreign and absent claims both return `missing`, with no run,
response or error. The public response contains no actor/request digest or request body.
When a run is known, the ordinary run-prefix authorization also applies.
POST replay rechecks the current control/live scopes and the retained run prefix, without
calling external capability or admission ports. Losing live scope still permits an
otherwise authorized read-only recovery, but cannot authorize a submission replay.

| State | Meaning | Client action |
| --- | --- | --- |
| `missing` | No claim visible to this subject | Preserve and use only the original request ID and exact payload; a conflicting hidden/legacy claim still refuses admission |
| `pending` | The owner accepted the claim; work or outcome persistence has not settled | Poll the same identity; inspect the run if present; do not initiate a fresh launch |
| `submitted` | The original acknowledgement is durably retained | Attach the returned run and read its current status; the retained acknowledgement is not a current progress snapshot |
| `failed` | The original refusal is retained, possibly after a durable run reservation | Preserve its class/code and inspect the reserved run when present; this record does not authorize a new-ID retry |

Completion persists its outcome before returning. A failure to persist the outcome
leaves the claim pending and returns write uncertainty. Existing stop, release and
reservation-persistence failure codes remain visible; they are not rewritten as
successful cancellation. A repeated failed reserved submission cannot reopen a session.
A pre-reservation refusal is cached only when the store confirms no reservation and
persists the refusal. A failed lookup or outcome write retains a pending claim instead.

This route covers V1 submissions only. V2 seed submissions continue to use their
existing actor-bound operation/binding journals and recovery routes. Other mutations
retain their own command or operation identities; the new dispatch error must not be
interpreted as a generic retry policy for them.

## Storage and consumer compatibility

The file adapter accepts old records with an absent `submission_claims` map and writes
the new map on mutation. Old strict file readers reject the added field. SQLite adds
the bounded `management_submission_claims` table without changing existing run/event
rows or the execution-store migration version. Do not run an older management writer
against the upgraded SQLite database: it does not know the new pre-port claim fence.
Back up durable stores before an owner upgrade; rollback requires offline preservation
and reconciliation of all claims, not dropping the table or deleting pending records.

The closed schemas and shared vectors live in `contracts/management-recovery/`.
They belong to Harness; no neutral protocol artifact or Rust protocol version changes.
Consumer adoption must pin these exact producer bytes and test unknown-version handling,
pending/late completion, foreign-request hiding and original-ID retention before an
integrated or deployed recovery claim. The producer change alone does not establish that
Studio or any deployed client has adopted this contract.

## Evidence boundary

Tests force stalls before any external validation result and after a durable live run
reservation. They check the actual loopback HTTP response, retained capacity, read-only
recovery, actor isolation, late completion, no second port invocation, explicit shutdown
uncertainty, immutable outcomes and restart/racing SQLite claims. Repeated shutdown and
dropped-handle regressions prove that the same supervisor retains accepted capacity and
joins every released handler. A capacity-one registry test proves that abandoning its
handle cannot admit a replacement until the original supervisor is joined. Served stalls
cover read-only POST handlers and GET; disconnected-channel tests exercise the same
production outcome mapper for reads, writes and unknown route shapes. All ports and gameplay
are synthetic. This does not reproduce or close Studio's intermittent native 502 cause,
prove native/provider latency, or establish cross-process cancellation of a stalled port.
The HTTP wire regression covers every legacy class mapping, including Studio's existing
`inference_profile_context_incompatible` refusal, alongside the two recovery-specific
class overrides; retained owner errors remain separately classified.
