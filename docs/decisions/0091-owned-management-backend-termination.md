# ADR 0091: Terminate stalled management backends under an owned Linux process scope

## Status and scope

Accepted engineering decision for the unreleased Linux `sts2-workflow serve` and
`sts2-harness-runtime serve-workflow` commands. Refs
[#818](https://github.com/AI-Ascension/sts2-harness/issues/818).
This extends [ADR 0089](0089-bounded-management-dispatch-recovery.md); its actor-bound
claims, closed error schemas, replay authorization and existing mutation identities
remain authoritative.

The generic `ManagementServer` embedding API still accepts arbitrary synchronous Rust
ports and retains their threads until they join. A caller-provided `Arc` cannot be
serialized into a separate process, and Rust offers no safe arbitrary thread kill.
That API continues to provide bounded waits and retained supervision. It does not gain
a stalled-thread termination guarantee from this command-specific composition.

The shipped commands require Linux process and procfs facilities. They refuse the
owned transport on other platforms; there is no silent fallback with weaker cleanup
semantics. This does not qualify a Windows job-object implementation or extend the
existing Windows worker credential/IPC boundary.

## Process ownership

The same approved executable has three roles. Each role receives the original command
and owner configuration; only private inherited sockets carry HTTP payloads between
them. No request supplies an executable, command line, endpoint or filesystem path.

| Role | Owned state | Work it may execute |
| --- | --- | --- |
| HTTP frontend | Listener, bounded connections, one helper `Child`, bounded private queues | Parse/write bounded HTTP, classify request effects, queue diagnostics, await typed private replies |
| Dedicated supervisor helper | One backend generation and all descendants adopted by its Linux subreaper scope | Enforce absolute deadlines, terminate/reap its children, refuse replacement while exit is uncertain |
| Persistent backend | Authentication, service, stores, context owner, runtime/provider ports and bounded handler threads | Dispatch the actual authenticated route and write diagnostics through the configured stderr |

The frontend starts the helper before admitting HTTP. The helper composes a backend and
waits for its ready frame before the frontend binds the public loopback listener.
The frontend and embedding host never become subreapers and never call an all-child
wait. The helper establishes its exclusive scope before creating children; a preexisting
child causes refusal without consuming its exit status or signalling it.

The helper first terminates and reaps the direct backend using its retained `Child`.
Only after that handle caches the exit status does the helper inspect adopted children.
It binds a child to its parent PID, birth time and UID, opens a pidfd, and rechecks the
identity before signalling the retained descriptor. It can therefore drain a descendant
that has escaped the backend's process group without relying on a reusable numeric PID.
Only kernel `ECHILD` proves a complete drain. An empty procfs scan never proves absence.

Each scan admits at most 65,536 procfs entries and 4,096 owned children; individual stat
reads are bounded at 4,096 bytes. Deadline checks occur within enumeration, identity
inspection, signalling and reaping. If the kernel cannot establish termination within
the reap budget, the helper remains the owner and keeps the generation quarantined.
It continues draining and cannot create a replacement. This is an explicit pending
outcome, not a claim that an uninterruptible kernel task, hostile host, unavailable
procfs or arbitrary operating-system failure can be made to disappear on a timer.

All fallible socket/frame preparation precedes spawning each child. After a successful
spawn, an owned generation or helper stores its `Child` before any further fallible
operation. Closing the frontend's private channel initiates the same backend drain.
Normal frontend teardown waits for the actual helper exit and joins accepted connection
workers. It never drops a live helper in order to report successful cleanup.

The dedicated helper installs its own payload-free panic hook before supervision. This
does not modify the embedding host's hook. The default hook could block on inherited
stderr before `catch_unwind` reached the drain, so helper panic reporting uses a fixed
private `SupervisorFailed` code after actual cleanup instead. A backend panic remains
inside the helper's termination boundary. Abort, external supervisor kill, machine loss
and hostile host interference are outside the orderly-cleanup guarantee.

## Independent bounds and private identity

| Phase or resource | Bound and behavior |
| --- | --- |
| HTTP request read | Existing five-second absolute budget; headers, path and JSON remain bounded |
| Dispatch | Separate default 30 seconds; `STS2_MANAGEMENT_DISPATCH_MILLIS` accepts a positive integer through 300,000 milliseconds |
| Backend startup | 30 seconds per generation; no public listener is admitted before initial readiness |
| Initial termination/reap attempt | Two seconds after generation retirement; pending ownership remains explicit if proof is unavailable |
| Frontend dispatch response wait | Original dispatch budget plus bounded reap/IPC scheduling allowance; late replies cannot attach to another request |
| HTTP response write | Fresh five-second absolute budget; each partial write recomputes remaining time and retains its cursor |
| Diagnostic write | One second inside the backend; no frontend lifecycle/failure sink performs synchronous stderr I/O |
| Handler admission | At most 16 by default; when capacity exceeds one, one slot remains reserved for exact authenticated submission-recovery GETs |
| Diagnostic admission | Separate bounded queues and at most 64 diagnostic workers; overflow increments explicit diagnostic loss |
| Private framing | At most 96 queued frames and a separate byte cap of 16 maximum-size frames; each I/O turn transfers at most 64 KiB without blocking |

One Linux monotonic absolute deadline crosses every private hop. Serialization, queuing
and backend startup do not reset it. The backend checks the same deadline again before
starting a route. The helper retains dispatched work until a response or actual generation
retirement settles its local execution state. Frontend timeout alone is not cancellation.

Private frames use an inherited unnamed Unix socket and closed tagged JSON envelopes.
Each hop checks protocol version, generation and monotonically increasing sequence.
Bodies and decoded HTTP bounds are rechecked. An invalid identity or malformed frame
fails closed and drains the owned backend. Late responses to an already-timed-out
sequence cannot become another request's answer. Capacity refusal before dispatch is
typed separately from an accepted handler with unknown outcome.

The shipped management client retains its default 45-second overall wait. An operator
who increases the server dispatch budget must also choose an appropriately bounded
consumer wait. A shorter client timeout does not authorize a new operation identity.

## Outcomes, durability and restart

At a dispatch deadline, the helper retires the entire backend generation. After actual
reap, a possibly mutating request receives HTTP 503 with owner-local v2
`write_state_unknown` and code `dispatch_backend_terminated`. If termination is still
pending, it receives `dispatch_termination_pending`. These codes describe local handler
execution; neither proves that an external provider, gateway or game effect was undone.
Exact read-only routes retain v1 HTTP 503 `unavailable`, including the reviewed POST
validation/inspection/diff/preflight/replay/export/receipt routes. Unknown route shapes
remain conservatively possibly mutating. Existing v1 status-derived classes are unchanged.

An expired request which never reached backend admission receives `dispatch_not_started`
with `unavailable`. Admission refusal does not establish that some earlier request with
the same operation identity never wrote. All clients still recover their original identity.

Only the backend opens owner stores and process-lifetime journal locks. Replacement
begins after all old backend descendants are reaped, so lock release does not depend on
executing Rust destructors. No lockfile is deleted and no journal is rewritten to force
recovery. Durable pre-port submission claims survive termination: a pre-reservation claim
can remain pending without a run; a post-reservation claim retains its original run.
Matching replay never relaunches, changed payloads conflict, and foreign actors cannot
learn the claim's existence through its recovery route.

The backend is shared. Retiring it for one stalled request also loses every other
in-memory live session in that generation. A retained running snapshot may therefore
report `NeedsOperator` after replacement. Recomposition does not resume that session,
reopen its provider, or silently submit a replacement run. Historical context receipts
remain history; they do not reinstall a current association or writer authority. Existing
owner epochs, command/operation receipts and context-control fences continue to govern
recovery. Native game lifecycle, save-profile or external effect reconciliation remains
with its existing gateway/MCP/owner paths.

## Diagnostics

Normal lifecycle markers retain their route templates, budget, per-server identity,
elapsed time, status and redaction. They are queued without blocking the HTTP frontend.
The backend acknowledges a marker only after the actual stderr write. A full queue,
refused diagnostic or retired generation increments a saturating frontend loss counter.
Every owned HTTP response includes `X-Ascension-Diagnostic-Loss`, the cumulative loss
known when that response is serialized. A later loss may first appear on a later response;
the header is not an acknowledgement of that response's terminal marker.

An empty or incomplete stderr stream alone is therefore no longer evidence that no
request arrived or that all diagnostics succeeded. Check the loss counter and typed
response. Diagnostic loss does not expose tokens, request bodies, request identifiers,
private provider content or panic payloads. A blocked diagnostic can retire the shared
backend and invalidate its live sessions just as another stalled handler can; this cost
is explicit rather than an unbounded dependency of the public HTTP path.

## Verification and limits

The process regressions use actual sockets, SQLite stores, child processes and pidfds.
Their workflow/game/provider ports are controlled fixtures. They exercise permanently
parked handlers, escaped process groups, pre/post-reservation recovery, ordinary-capacity
saturation with reserved recovery, other live sessions becoming `NeedsOperator`, shutdown,
blocked stderr, helper panic reporting, malformed framing and stale private identity.
The shipped `sts2-workflow serve` command also faces a real competing SQLite transaction:
the blocked backend exits within its dispatch/cleanup bound, its uncommitted claim remains
missing, and subsequent original-ID recovery precedes one explicitly requested admission.
The existing receipt-restart and authorization suites remain necessary alongside them.

This proves the tested local Linux transport/ownership boundary. It does not establish
native or paid-provider latency, external rollback, Windows support, consumer deployment,
or the cause of Studio's earlier intermittent 502. No shared protocol artifact, copied
golden vector or dependency version changes. Consumers still need to adopt and test the
existing Harness error/recovery schemas at an exact producer source pin.
