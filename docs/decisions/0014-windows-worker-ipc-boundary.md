# ADR 0014: Isolated Windows worker IPC boundary

## Status and approval gate

Proposed for independent transport/security review under the authorized watchdog
implementation assignment. This document does not approve FFI implementation by
itself. Record reviewer acceptance and root integration approval before adding
the boundary. The existing workspace-wide unsafe-code prohibition remains intact.

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

## Narrow unsafe boundary

Only this separately reviewed package may contain documented native FFI. Harness
policy, worker admission, schemas, credential comparison and execution stay in
safe Rust with unsafe forbidden. Every FFI call must document pointer lifetime,
buffer length, handle ownership, synchronous/overlapped I/O lifetime and error
handling. Owned handles close exactly once, including partial-construction errors.
No public API accepts arbitrary pointers or transfers unchecked raw handles.

The endpoint is local-only and first-instance protected. Its DACL permits only
the configured worker and watchdog principals; remote clients and broad ACLs are
rejected. Peer capture uses kernel-provided pipe identity, a retained limited-query
process handle, creation-time and SID checks, and the approved image digest.
Credential files are regular protected files, opened through held protected
ancestors with no reparse traversal or replacement window. Credentials are bounded,
never logged/serialized and never accepted from command-line arguments.

Each connection has one bounded absolute deadline, finite client/frame limits and
owned cancellable I/O. Partial I/O cannot extend the budget. Cancellation completion
must precede releasing overlapped buffers/handles. Authentication precedes request
decoding, store access and admission. The frozen handoff JSON remains unchanged;
the versioned authentication prelude is a separate transport contract.

## Required evidence

Before integration, independently review source and execute native Windows
synthetic tests for wrong SID/PID/creation time/image, peer death, unauthorized
clients, reparse/ACL/path replacement, stolen endpoint names, malformed/oversized
preludes, slow readers/writers, cancellation and handle cleanup. Test-only children
are explicitly owned; no service, game, account security or unrelated process is
changed. Cross-compilation is only build evidence, never native authentication
evidence. Linux uses safe owner-local APIs and does not inherit this exception.
