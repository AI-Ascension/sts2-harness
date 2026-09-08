# ADR 0014: Isolated Windows worker IPC boundary

## Status and approval gate

Approved for isolated original authoring under the authorized watchdog assignment,
following independent H40/H43 design acceptance and the root approval recorded in
[ADR 0015](0015-worker-transport-authoring-contract.md). This is not integration
approval: independent source review and native fault evidence remain mandatory.
The existing workspace-wide unsafe-code prohibition remains intact.

## Need and ownership

The Windows harness worker must authenticate its watchdog peer before admitting
control or job requests. A PID alone is insufficient: the server must retain a
process handle, validate its creation time, permitted SID and approved executable
identity, and reject a dead/replaced process. Safe named-pipe libraries supply
some transport operations but have not established this complete held-identity
and protected-credential contract.

The proposed non-empty, Windows-only `worker-ipc-windows` package belongs to this
repository. It owns only local named-pipe transport, protected credential reads
and peer-identity verification. It has no game/host access, gameplay authority,
process launch/termination, scheduling, provider behavior or database access.
It does not depend on or copy the watchdog's platform implementation.
The sole fatal self-termination exception, with no authority over another process,
is the supervised-worker containment amendment in
[ADR 0016](0016-worker-native-io-fail-stop.md).

## Narrow unsafe boundary

Only this separately reviewed package may contain documented native FFI. Harness
policy, worker admission, schemas, credential comparison and execution stay in
safe Rust with unsafe forbidden. Every FFI call must document pointer lifetime,
buffer length, handle ownership, synchronous/overlapped I/O lifetime and error
handling. Owned handles close exactly once, including partial-construction errors.
No public API accepts arbitrary pointers or transfers unchecked raw handles.
The native implementation must additionally state alignment, UTF-16 termination
and length units, error-code capture, and thread-safety invariants. Unsafe calls
remain private to the platform module behind typed safe wrappers.

The endpoint is local-only and first-instance protected. Its DACL permits only
the configured worker and watchdog principals; remote clients and broad ACLs are
rejected. Peer capture uses kernel-provided pipe identity, a retained limited-query
process handle, creation-time and SID checks, and the approved image digest.
Inherited or ambiguous access rules also fail closed. The expected peer process
comes from the approved launch relationship, never from a client-supplied PID.
Credential files are regular protected files, opened through held protected
ancestors with no reparse traversal or replacement window. Credentials are bounded,
never logged/serialized and never accepted from command-line arguments.
Reject hardlinks and remote/device paths; environment variables cannot contain
credential bytes. Keep secret buffers private and erase them on release using a
reviewed mechanism; returning raw credential bytes is not a public API.

Each connection has one bounded absolute deadline, finite client/frame limits and
owned cancellable I/O. Partial I/O cannot extend the budget. Cancellation completion
must precede releasing overlapped buffers/handles. Authentication precedes request
decoding, store access and admission. The frozen handoff JSON remains unchanged;
the versioned authentication prelude is a separate transport contract.
That contract must specify connection/session binding and replay handling before
transport integration. A reusable credential alone is not evidence of a fresh
worker boot or permission to revive an old control mode. Shutdown rejects new
connections, cancels outstanding I/O, and joins every owned task before returning.

## Review stages

The independent H40 design review accepted this ownership scope, and H43 accepted
the concrete authoring gate subject to precision clarifications now recorded in
ADR 0015. Root approves only isolated original implementation against that API,
dependency plan and invariant set. No workspace-wide lint relaxation is permitted.
Source review and executable native fault tests necessarily follow implementation;
both must pass before platform integration or a Windows security claim. Missing
native evidence does not permit treating cross-compilation as acceptance.

## Required evidence

Before integration, independently review source and execute native Windows
synthetic tests for wrong SID/PID/creation time/image, peer death, unauthorized
clients, reparse/ACL/path replacement, stolen endpoint names, malformed/oversized
preludes, slow readers/writers, cancellation and handle cleanup. Test-only children
also cover PID reuse, remote clients, inherited/broad ACLs, hardlinks, deadline
expiry in each phase, cancellation races and partial-construction handle cleanup.
These children
are explicitly owned; no service, game, account security or unrelated process is
changed. Cross-compilation is only build evidence, never native authentication
evidence. Linux uses safe owner-local APIs and does not inherit this exception.
