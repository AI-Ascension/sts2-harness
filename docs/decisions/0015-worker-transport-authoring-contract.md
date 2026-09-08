# ADR 0015: Worker transport authoring contract

## Status and gate

Approved for isolated original authoring following independent H40/H43 design
acceptance of this contract and [ADR 0014](0014-windows-worker-ipc-boundary.md).
Root records authoring approval with the review's precision clarifications below.
This is not permission to integrate an unverified native adapter, enable an
endpoint, or claim Windows security.

## Safe API and lifecycle

The Windows-only package exposes validated `EndpointPolicy`, `ExpectedPeer`,
`WorkerListener`, `AuthenticatedConnection`, and redacted `TransportError` types.
Construction is fallible. Fields carrying native resources or secrets are private;
there is no public raw-pointer/handle conversion or credential-byte accessor.
Errors expose only fixed categories: configuration, identity, credential, deadline,
framing, busy, closed and OS failure. They never include paths, PIDs or OS messages.

`WorkerListener::bind(policy)` creates the first protected local pipe instance.
`accept_authenticated(deadline)` returns an authenticated connection or a typed
failure. That connection permits exactly one bounded request and one bounded
response, then closes; it cannot authenticate a second prelude or resume a partial
exchange. `shutdown` rejects new connections, cancels and joins outstanding I/O,
and releases owned handles. Drop is a final resource cleanup, not a detached task.

The native package authenticates transport only. It returns bounded request bytes
with an opaque verified-peer witness, not a decoded command or admission decision.
Safe harness code performs the existing closed decoder, capability, worker/watchdog
boot, immutable tuple and durable-control checks before accessing execution state.
The witness cannot be fabricated outside the package, serialized, or reused for a
different connection. No endpoint method executes jobs or accesses an owner store.

## Endpoint and approved peer

The owner-approved launch configuration supplies the exact watchdog SID, PID,
creation token, immutable executable identity/digest, and launch nonce. These values
are not read from client frames or inferred from executable names. The worker SID
is separately configured and checked against its own process token. The endpoint
name contains a fixed product prefix and a canonical UUIDv4 launch nonce, is local
only, and is not a path supplied by a remote request.

`ExpectedPeer` is constructed only by safe owner-launch configuration loading,
before listener creation, from the approved supervisor's launch record. Its
immutable lifetime is one worker launch; replacement requires a new launch record
and listener, not a client update. On Windows the creation identity is the exact
unsigned 64-bit `GetProcessTimes` creation FILETIME (100-nanosecond units since
1601-01-01 UTC), compared through the retained process handle opened for the
kernel-reported pipe-client PID. It is not a wall-clock estimate or Linux token.

The server creates and holds the first-instance pipe, rejecting stolen names.
Its explicit non-inherited DACL admits only the two configured principals, deduped
when identical; it has no broad group or remote access. Kernel-reported client PID,
token SID, held process creation identity, liveness and approved image must match
before credential admission. The client must independently authenticate the server
before sending its credential. A process handle remains held through the exchange.
The watchdog companion owns that client-side verification; this server package
cannot supply it. Cross-consumer acceptance must test both obligations.

Open the approved executable through protected non-reparse ancestors as a regular
non-hardlinked file, with sharing that denies write and delete. Hash the held bytes
before readiness and retain the image/ancestor handles through every exchange.
Compare the peer's kernel-reported full image to the held file's volume serial and
128-bit file ID, opening the reported image through the same protected-path rules.
A matching path string alone is insufficient. Any sharing or identity conflict
fails closed; neither a cached digest of replaceable bytes nor rehash-after-trust
is accepted.

The initial endpoint supports one active exchange and one listening instance,
with no in-process waiting queue and bounded busy rejection. The transport must
release its exchange before provider execution: control cannot wait for inference.
Increasing concurrency requires a separately bounded policy and tests.

## Transport authentication profile

The cross-consumer profile is `ascension-worker-auth-v1`, owned by the watchdog
handoff boundary; the harness implements its own consumer. It preserves the frozen
handoff JSON and schema digest. Each fresh OS-authenticated connection carries:

1. A four-byte unsigned big-endian length and the exact 25-byte magic
   `ascension-worker-auth-v1` followed by NUL, then 1..4096 credential bytes.
2. One separately framed handoff JSON request, limited to 65536 bytes and nesting 16.
3. One separately framed handoff JSON response, with the same limits, then close.

The auth length counts magic, its terminating NUL, and credential bytes: exactly
26..4121 bytes. Reject the prefix outside this range before allocating/reading the
body. JSON prefixes count only their UTF-8 body, 1..65536 bytes. Prefix/body error,
timeout or partial cancellation poisons and closes the connection, following
`worker_frame_io`; no later frame may resynchronize it.

The magic length is validated against its actual encoded bytes during implementation;
its SHA-256, including the terminating NUL, is
`d6898d0c5866f61e7ed225117542da7d7641ac44e93f5c4d819a5927663c970d`.
There is no auth echo or positive credential-only response. Wrong magic, empty,
oversized or malformed credentials close the connection before JSON decoding.

The reusable credential is not a cryptographic freshness proof. This profile relies
on a protected OS connection bound to the exact approved live peer process and
held creation identity. It cannot be enabled on TCP or an unauthenticated pipe.
The launch nonce binds endpoint selection, not a client-chosen proof. Captured auth
bytes from another process or launch fail the OS identity/endpoint checks.

After transport admission, the request's fresh UUIDv4 correlation and watchdog boot
are validated. Probe is read-only and deliberately has no target worker boot. Every
other command must name the current worker boot. Replay of an old boot is rejected;
same-boot dispatch/lookup/acknowledgment retain the same durable tuple and terminal
digest, while control uses persisted monotonically checked mode sequence and exact
same-sequence idempotency. Replaying a credential cannot install authority, reverse
stop, allocate a new attempt, or turn a lookup into dispatch. A compromised already
authorized peer is inside the transport trust boundary; durable capability and
fencing checks still apply. No claim of network challenge-response security is made.

Probe, lookup, acknowledge, dispatch and control scopes map only to their matching
closed command. A dedicated worker-control credential has this bounded surface;
operator/admin credentials and gameplay leases are never interchangeable.

## Credential and filesystem policy

The owner-approved configuration supplies a fixed dedicated credential path, never
credential bytes in argv, environment, frames other than the private auth prelude,
logs or store records. Credentials contain 1..4096 printable ASCII bytes excluding
space; malformed files are rejected rather than trimmed or normalized.

Open and hold protected ancestors and a stable regular-file descriptor/handle.
Reject symlinks, reparse points, hardlinks, device/remote paths, ambiguous owners,
inherited/broad write access and replacement windows. POSIX opens use nonblocking
flags before type inspection so a FIFO substitution cannot hang before validation.
That POSIX requirement belongs solely to the separate safe Linux adapter and does
not expand this Windows package's native scope.
Windows opens preserve immutable file identity through held sharing restrictions.
The approved executable is similarly held against replacement when its digest is
validated. Startup preparation is separately bounded and is not connection readiness.

Keep the secret in a private zeroizing buffer, with no Debug/Clone/serialization
implementation. Compare fixed-size padded bytes and encoded length using a reviewed
constant-time primitive; never return the candidate or stored secret. Erase candidate
buffers on every failure and after authentication. Native buffers holding secrets
must not outlive their owning I/O completion. Credential rotation is explicit owner
configuration/restart, not automatic reload during recovery.

## Deadline and native cancellation

Create one local monotonic deadline of at most five seconds before accept and use
it through peer checks, credential admission, framing and response. Preparation
must not hide unbounded filesystem reads inside an accepted connection. Partial
I/O never extends the deadline; the request may only shorten it. The existing safe
`worker_frame_io` budget can be used where the native wrapper supplies compatible
cancellable I/O; its duplex tests do not prove Windows overlapped cancellation.

Native I/O owns its OVERLAPPED storage and buffers until successful completion or
confirmed cancellation completion. Cancelling is not completion. No detached
blocking thread may retain credentials, pipe handles or buffers after shutdown.
Validate pointer alignment, byte/UTF-16 units, termination, borrowed lifetimes,
thread use, error-code capture and exactly-once close for each unsafe operation.

## Dependency and lint plan

The proposed pins are crates.io `windows-sys = 0.61.2` (MIT OR Apache-2.0,
Microsoft windows-rs), `zeroize = 1.9.0` (Apache-2.0 OR MIT, RustCrypto utils),
and `subtle = 2.6.1` (BSD-3-Clause, dalek-cryptography subtle). Their local registry
metadata and the latter two license texts were inspected; this is a dependency
proposal, not an advisory-audit result. No package source is vendored.

Use Windows-only target dependencies and the narrow Foundation, Security/
Authorization, FileSystem, Pipes, IO and Threading feature groups required by
actual calls. `windows-link = 0.2.1` and `windows-sys` are already locked; secret
handling packages need explicit root-owned lock updates and notices. No derive,
network, provider or unrelated Windows features are requested. Cargo metadata,
license/source checks and a full-lock advisory check must accompany those updates.

The initial exact feature allowlist is `Win32_Foundation`, `Win32_Security`,
`Win32_Security_Authorization`, `Win32_Storage_FileSystem`, `Win32_System_Pipes`,
`Win32_System_IO`, and `Win32_System_Threading`. An additional feature needs a
named API and review. `zeroize` disables default features and enables `alloc`;
`subtle` disables default features. No derive or nightly dependency is enabled.

The new package forbids unsafe in its safe entry/API modules and allows it only
in a private reviewed native module. The existing workspace `unsafe_code = forbid`
and harness/core/Linux policies remain unchanged. Any package-local lint exception
must be exact and recorded under ADR 0014, never inherited by other packages.
Its package-local Rust lint is `unsafe_code = "deny"` with
`unused_must_use = "deny"`; only the private native module has a scoped allow.
All safe API/policy modules separately use `#![forbid(unsafe_code)]`. Its Clippy
policy retains warnings plus denied expect/panic/todo/unimplemented/unwrap. This
package does not inherit and then weaken the workspace forbid; existing packages
continue inheriting it unchanged. Package/source review must verify this layout.

Independent source review and the native Windows fault matrix in ADR 0014 remain
mandatory before integration. Root authoring approval is not root integration
approval, service installation, live recovery or release evidence.
