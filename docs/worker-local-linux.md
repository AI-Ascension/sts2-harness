# Linux worker transport and executable candidate

This document describes the Linux-only transport candidate in
`crates/harness/src/worker_local_linux.rs` and its opt-in executable command loop.
This is an implementation candidate, not a released service or live recovery claim.

## Owned boundary

The adapter owns one protected Unix stream listener, one held protected
credential file, Linux peer proof, the authentication prelude, and bounded
request/response framing. It does not decode the frozen handoff JSON, access a
database, admit a job, launch or stop a process, or perform a gameplay action.
After authentication it returns bounded request bytes together with an opaque
`LinuxPeerWitness`; the parent harness server remains responsible for request
decoding, capability checks, boot/tuple validation, durable admission and
response construction.

## Filesystem and endpoint proof

The endpoint, credential, and approved-image paths must be absolute, canonical
component paths. Every existing ancestor is opened with `O_DIRECTORY |
O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK`, checked as a protected directory, and
held for the relevant listener or peer-exchange lifetime. The endpoint name is
never removed before bind and is opened/held after bind with `O_PATH |
O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK`; device/inode identity is checked before
every accept and on cleanup. A socket replacement is rejected and cleanup
removes only the identity created by this listener.

The credential is opened through the held parent with `O_RDONLY | O_NOFOLLOW |
O_CLOEXEC | O_NONBLOCK` before regular-file, owner, permission, link-count and
size checks. The descriptor is held while the bounded printable-ASCII secret
is read and the path identity is checked again. FIFOs, devices, symlinks,
hardlinks, writable credentials and replacement races fail closed. Credential
bytes are private zeroizing buffers and never occur in errors, logs, argv,
environment, frames after authentication or durable records.

The approved executable is opened and hashed from its held descriptor before
the listener is ready; its protected ancestors, regular-file identity and
owner-approved digest remain held by the listener. Release artifacts with any
owner, group or other write bit are rejected. Each accepted peer first checks
the exact kernel-reported `/proc/<pid>/exe` path, then follows only that
generated proc magic link with a read-only descriptor. The descriptor itself is
type-checked and compared by device/inode with the approved held image. The
accepted-peer path does not rehash that live descriptor: the SHA-256 digest was
established for the owner-approved image when the listener opened it, and
device/inode equality binds the live proc image to that held artifact for this
point-in-time proof. A second descriptor identity check and path check catch an
exec transition during proof; a path string alone is never proof. The proc
magic-link follow is deliberately not used for caller-provided paths.

The bounded image hash performed while opening the owner-approved image is
current-byte proof for that point in time, not continuous loaded-code
attestation. Open/stat/read operations are synchronous and bounded in bytes but
are not preemptible by Tokio; the adapter checks the absolute deadline before
and after each operation and rejects if it has elapsed, while honestly allowing
a syscall already in the kernel to return after that instant. No detached or
unbounded blocking worker is created.
`O_NONBLOCK` prevents a FIFO substitution from hanging before type validation,
but does not make ordinary regular-file reads cancellable.

## Peer and wire proof

The configured peer is an owner-approved UID/PID, `/proc/<pid>/stat` start
token, executable path and SHA-256 image digest. Every accepted stream obtains
`SO_PEERCRED`, opens and holds a pidfd, checks pidfd liveness, the start token,
and executable path/image identity around peer proof before credential
admission. Mismatches are rejected before reading the credential. The held
pidfd and approved/live image descriptors are the non-forgeable witness
lifetime. This remains a point-in-time configured-principal check: a held
pidfd does not prevent a trusted process from later calling `exec`, and the
transport makes no root-compromise or continuous-attestation claim.

The listener admits one exchange at a time. A concurrent accept returns a fixed
`Busy` error immediately; there is no application-level waiting queue. Dropping
an in-progress accept releases that exchange reservation.

The auth profile is exactly:

```text
u32be(total_body_bytes)
ascension-worker-auth-v1\0       (25 bytes)
credential                       (1..4096 printable ASCII bytes, no space)
```

`total_body_bytes` is bounded to 26..4121 and covers the magic plus
credential. There is no auth echo. A successful connection then carries one
existing 4-byte-big-endian/65536-byte handoff request and one response under
the same absolute `ConnectionDeadline`; partial I/O, cancellation and any
failed phase close the connection and cannot be resumed.

## Executable integration

With `STS2_WORKER_MODE=true`, Linux worker startup reads the bounded private-pipe
bootstrap and persists a fresh stopped worker boot. In addition to the existing
worker/runtime policy, the approved launch environment must provide:

| Setting | Meaning |
| --- | --- |
| `STS2_WORKER_ENDPOINT` | Protected absolute Unix socket path; an existing name is rejected |
| `STS2_WORKER_CREDENTIAL_PATH` | Protected absolute credential file path, never credential bytes |
| `STS2_WORKER_TIMEOUT_MS` | Canonical decimal integer from 1 through 5000; one full exchange budget |

The bootstrap component must be `harness`; the exact expected live watchdog
process comes only from that bootstrap, not from command JSON or environment.
Runtime/provider settings and workflow selection are captured before binding.
The existing runtime configuration digest is checked against the actual MCP bytes
and configured runtime policy before the endpoint is bound, and again at execution
attachment. That digest includes the configured run/episode/trajectory lineage;
the executable test explicitly approves its dispatched lineage. This change does
not establish generic multi-job reuse with different approved configurations.
Tokio's Linux-only `net` and `signal` features provide local transport and
SIGTERM/SIGINT ownership; no new dependency package is introduced by signal support.

The native exchange authenticates before decoding. The core persists admission,
writes the correlated response, crosses the running fence, and then transfers the
one-use execution task to one scoped thread. Failed authentication or client I/O
does not start work or restart the worker. Stop/pause and historical commands stay
on the control loop during execution. A completed thread is joined before its
durable completion can release capacity. Execution errors keep the lane fenced.

SIGTERM/SIGINT fence new admission and retain an active handoff as UNKNOWN before
draining and closing. They do not modify the watchdog's deployment desired state;
operator stop must first be persisted by the watchdog owner. An authenticated
worker `stopped` control leaves the command endpoint available for diagnostics.
See [ADR 0016](decisions/0016-owned-worker-execution.md) for the remaining hard
shutdown/descendant-containment gaps. The companion watchdog's static launch mapping,
cross-consumer executable tests and independent acceptance remain integration gates.

Authentication is a point-in-time process-principal and artifact check, not
continuous loaded-code attestation. A held pidfd does not prevent exec, fork or
descriptor inheritance by an already-authorized process. Such compromised-peer
behavior remains inside the explicit trusted-peer boundary; durable capability
checks and execution-time fencing are still required. Independent review must
verify the kernel image-to-file binding and actual deadline behavior before
this candidate can be promoted or deployed.

## Evidence boundary

[`worker_server_entry`](../crates/harness/tests/worker_server_entry.rs) executes the
actual copied runtime binary with protected synthetic credentials and an exact
native parent identity. It checks invalid-policy refusal before bind, rejected
credentials, stopped startup, authenticated running/stop control, persisted stop
after SIGTERM, and stop/probe responsiveness while an owned execution thread waits
on a synthetic gateway allocation. The interrupted handoff remains UNKNOWN after
shutdown. These are executable synthetic checks, not real MCP/provider/gameplay
or a proof of forced cleanup during a hung provider call.

Source review and Linux synthetic transport tests can establish bounded safe
behavior and expected rejection categories. They do not establish systemd,
watchdog, provider, game-host, live recovery, or release behavior. Native
Linux tests must still exercise wrong UID/PID/start token/image, dead peers,
symlink and path replacement, credential permissions, malformed/oversized and
trickled preludes, deadline expiry in every phase, cancellation, pidfd
retention, and positive process identity against the integrated server.
