# ADR 0016: Supervised worker native I/O fail-stop

## Status and scope

Root approves isolated authoring following independent H62 source/design review.
This amendment applies only to the Windows IPC boundary in ADRs 0014 and 0015,
used inside the externally supervised harness worker. Source review and native
fault execution remain required before integration. No endpoint is enabled by
this decision. Workspace lint, permissions, wire schemas and process ownership
remain unchanged.

## Failure found

Cancellation is a request, not completion. An unexpected result-query error does
not prove Windows has stopped using an OVERLAPPED structure or its buffer.
Returning and releasing stack storage on that path is unsafe. Conversely,
unbounded completion or condition-variable waits cannot satisfy finite shutdown
claims. Independent H62 rejected both behaviors in the implementation draft.

## Approved containment

One private operation owner retains stable OVERLAPPED storage, buffers and a
manual-reset event until completion is proven. Connect, read and write use the
same completion classifier. Only documented terminal operation results qualify;
unknown query/cancel errors remain unproven. `ERROR_NOT_FOUND` from cancellation
still requires checking completion. No operation owner may be released merely
because a cancellation request, disconnect, timeout or query error occurred.

After the ordinary absolute deadline, request cancellation and disconnect, then
use bounded waits within a separate cleanup grace of at most one second. This
grace is cleanup only: no additional application bytes, admission or response
success may be accepted. A close/cancellation generation wins over concurrently
completed input so a closed exchange cannot subsequently admit its request.
Shared operation guards prevent shutdown from releasing active handles. Every
condition-variable wait also uses the finite cleanup grace.

If completion remains unproven after the grace, invoke a private direct
`std::process::abort()` fail-stop. This path must not return, unwind, invoke
destructors, release operation resources, or call a configurable callback.
It is not successful cleanup. No arbitrary PID, external process termination,
launch, restart or scheduling API is introduced. This is the sole exception to
the package's prohibition on process termination: fatal self-abort to avoid
returning into memory-unsafe execution in its already-supervised worker.

Normal paths still zeroize secrets and close handles. User-mode destructors and
zeroization do not run on the fatal path; sensitive crash-dump policy belongs to
the approved protected host configuration. Do not enable dumps or emit secrets
for diagnosis. No detached task or leaked reusable service resource substitutes
for containment.

## External supervisor obligation

This does not guarantee bounded API return, successful shutdown, or OS process
termination on the fatal path. A hung native call may prevent even reaching the
local fail-stop. The watchdog remains the sole worker process owner and may
request termination through its exact held process identity. It must preserve
dispatch uncertainty and quarantine work if process exit cannot be established
within its cleanup bound. It must not launch a replacement or release an old
reservation based merely on timeout, abort intent or a termination request.

The launch/containment relationship and replacement suppression require an
integrated test before enabling this adapter. An independently reusable library
outside that relationship is not approved by this decision.

## Required fault evidence

Use a test-only native-call seam, never runtime-configurable fault injection:

- Cancellation not-found followed by confirmed completion.
- Normal completion racing cancellation, with closed exchange rejecting input.
- Unexpected cancel/query errors retaining the operation owner.
- Persistent incomplete/unknown results reaching child-process fail-stop after
  grace, without running operation-owner destructors first.
- Concurrent close, drop and begin with finite wait deadlines.
- An owned synthetic child blocked in a native-call simulation, with supervisor
  timeout retaining uncertainty and suppressing replacement.
- First-instance name theft during rearm failing closed; process liveness uses
  the held signaled-process state, not exit-code value 259.

Native tests must use finite owned children and exact cleanup identities. A
test proving local abort selection is not proof that a hung Windows driver
allows process termination. Cross-compilation is source/build evidence only.

## Primary references

- [Asynchronous I/O lifetime](https://learn.microsoft.com/en-us/windows/win32/fileio/synchronous-and-asynchronous-i-o)
- [Cancellation semantics](https://learn.microsoft.com/en-us/windows/win32/fileio/cancelioex-func)
- [Completion query](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getoverlappedresult)
- [Process termination limits](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess)
