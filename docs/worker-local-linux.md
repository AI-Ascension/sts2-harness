# Linux worker transport preparation

This document describes the Linux-only transport candidate in
`crates/harness/src/worker_local_linux.rs`. It is a preparation artifact, not
an enabled endpoint or a claim of native integration.

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
type-checked, compared by device/inode with the approved held image, and
hashed against the approved digest (at most 128 MiB) before it is retained in
the opaque witness. A second descriptor identity check and path check catch an
exec transition during proof; a path string alone is never proof. The proc
magic-link follow is deliberately not used for caller-provided paths.

The bounded image hash is current-byte proof for the point-in-time admission,
not continuous loaded-code attestation. Open/stat/read operations are
synchronous and bounded in bytes but are not preemptible by Tokio; the adapter
checks the absolute deadline before and after each operation and rejects if it
has elapsed, while honestly allowing a syscall already in the kernel to return
after that instant. No detached or unbounded blocking worker is created.
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

## Root integration contract

Root must review and apply the following integration changes separately:

- add a Linux-only module/export and wire it to the parent safe worker server;
- enable Tokio's `net` feature;
- add normal target dependencies pinned to `rustix = 1.1.4` with `event`,
  `fs`, `net`, and `process` features, `subtle = 2.6.1`, and
  `zeroize = 1.9.0`; update `Cargo.lock` and license/source policy together;
- pass a configured peer identity from the owner-approved launch record rather
  than deriving it from a request; and
- run the native Linux process fixture and fault matrix before enabling the
  endpoint.

Root dependency commits `5f299ae` and `935c889` provide the target-specific
manifest/lock and dependency-audit changes in this isolated candidate. Library
exports, runtime wiring, storage and Windows adapters remain unchanged.

Authentication is a point-in-time process-principal and artifact check, not
continuous loaded-code attestation. A held pidfd does not prevent exec, fork or
descriptor inheritance by an already-authorized process. Such compromised-peer
behavior remains inside the explicit trusted-peer boundary; durable capability
checks and execution-time fencing are still required. Independent review must
verify the kernel image-to-file binding and actual deadline behavior before
this candidate can be enabled.

## Evidence boundary

Source review and Linux synthetic transport tests can establish bounded safe
behavior and expected rejection categories. They do not establish systemd,
watchdog, provider, game-host, live recovery, or release behavior. Native
Linux tests must still exercise wrong UID/PID/start token/image, dead peers,
symlink and path replacement, credential permissions, malformed/oversized and
trickled preludes, deadline expiry in every phase, cancellation, pidfd
retention, and positive process identity against the integrated server.
