# Native worker endpoint v1

Status: source-derived Linux endpoint implementation. The endpoint is an adapter
around the harness-owned `WorkerRuntime`; it is not a second watchdog, gateway,
MCP, game, or provider authority. Non-Linux builds return an explicit unsupported
error until an equivalent platform adapter is reviewed.

## Launch contract

The watchdog launch helper writes one `ASC-WB01` bootstrap frame to the target's
exclusive standard input. The frame is a 12-byte prefix (eight-byte magic and a
big-endian `u32` payload length), followed by at most 16 KiB of strict JSON. The
closed payload contains exactly `version`, `launch_nonce`, `watchdog_boot_id`,
`component_id`, and `expected_peer`. The Linux peer contains the positive PID,
canonical `/proc/<pid>/stat` start token, absolute executable path, lowercase
SHA-256 image digest, UID, and GID. The endpoint rejects other versions, unknown
or duplicate fields, non-canonical UUIDs, unsafe paths, invalid tokens, and
non-lowercase digests before creating a listener. The bootstrap pipe must also
reach true EOF after the declared frame within the bootstrap deadline; a
delayed byte is not treated as an empty read.

Static owner-approved configuration is supplied through the environment:

| Variable | Meaning |
| --- | --- |
| `STS2_WORKER_ENDPOINT_NAMESPACE` | Existing owner-only directory in which the nonce-bound socket is created. |
| `STS2_WORKER_CREDENTIAL_PATH` | Owner-only regular file containing the local authentication secret. |
| `STS2_EXECUTION_STORE_PATH` | Harness-owned durable execution store. |
| `STS2_WORKER_DEPLOYMENT_ID` | Deployment identity (`STS2_DEPLOYMENT_ID` is an exact-value alias). |
| `STS2_WORKER_OWNER_ID` | Must equal the bootstrap component identity (`STS2_WORKER_OWNER` is an alias). |
| `STS2_WORKER_PROFILE_DIGEST` | Approved worker profile digest. |
| `STS2_WORKER_RELEASE_DIGEST` | Approved harness/release digest (`STS2_BUILD_DIGEST` is an alias). |
| `STS2_WORKER_CONFIG_DIGEST` | Approved runtime configuration digest (`STS2_RUNTIME_CONFIG_DIGEST` is an alias). |
| `STS2_WORKER_SEED`, `STS2_WORKER_STATE_DIGEST`, `STS2_WORKER_PROVIDER_DIGEST` | Components of the execution fingerprint; the legacy `STS2_SEED`, `STS2_STATE_DIGEST`, and `STS2_PROVIDER_DIGEST` names are exact-value aliases. |
| `STS2_WORKER_RUNTIME_BINARY` | Optional canonical executable to launch for an admitted handoff. Defaults to the current image, including a sealed Linux memfd image. |
| `STS2_WORKER_RUNTIME_SHA256` | Optional expected digest for that executable; otherwise the release digest is used. |

Aliases must agree when both are present. Values are never taken from a worker
request. The endpoint rejects empty/non-UTF-8 values, protected pseudo-filesystem
references, traversals, symlinks, writable executable images, and mismatched
digests. It copies only an allowlisted `STS2_`/basic process environment into an
admitted child and deliberately excludes endpoint, credential, runtime-image, and
timeout controls.

## Endpoint and authentication

The endpoint path is derived only after bootstrap validation:

```text
<namespace>/ascension-worker-<launch_nonce>.sock
```

The namespace must be an existing owner-owned directory with no group/other
permissions, no symlink, and no empty, dot, dot-dot, backslash, or control path
component. The derived path is bounded to 100 UTF-8 bytes. Binding is first-use
only: an occupied path fails closed and is never unlinked by name. A guard removes
the socket on exit only when its device/inode still match the bound socket.

Each connection is authenticated in this order:

1. The endpoint enables `SO_PASSCRED` and checks the connected PID, UID, and GID.
2. It obtains a PID file descriptor and rechecks the process start token and
   `/proc/<pid>/exe` path.
3. A bootstrap-time proof thread opens the peer image, checks its configured
   device/inode, hashes it once, and compares the expected digest. The held image
   and proof are reused by connections; a connection still rechecks current
   process/path/inode identity before accepting bytes.
4. The client sends a big-endian length-prefixed authentication body consisting
   of `ascension-worker-auth-v1\0` followed by the credential file bytes. The
   endpoint reads the credential only after peer authentication and compares the
   body in constant time.
5. Every received frame carries SCM credentials. File descriptors, duplicate or
   unsupported ancillary data, missing credentials, peer changes, and truncated
   control data are rejected.

Authentication has a fixed overall five-second frame deadline and at most four
in-flight authentication slots. Once those slots are occupied, the listener is
left unread so the kernel backlog supplies bounded backpressure; an incomplete
client cannot indefinitely serialize all subsequent connections.

The request and response use the frozen `worker_handoff` JSON codec and a
big-endian `u32` length prefix, bounded by `worker_handoff::MAX_FRAME_BYTES`.
The endpoint rechecks the peer before and after every received frame and around
each response write. Transport success is not effect settlement.

## Admission and execution

After authentication, `WorkerRuntime` owns the durable store and a capacity-one
execution lane. It handles probe, historical lookup, acknowledgment, and
operator-control commands through the existing command admission contract. For a
dispatch it validates the complete immutable tuple and execution fingerprint,
records the durable handoff, and sends the response before starting the child.
If the response cannot be written after admission, the handoff remains an
uncertain retained record; it is not silently retried.

Only an admitted reservation can start the configured runtime child. Before the
endpoint accepts the runtime configuration, it copies the verified executable
bytes into a sealed executable memfd. Every child is launched through that
retained descriptor, so pathname replacement or in-place source mutation after
startup cannot change the approved image. The child
receives `--resume`, the exact run/episode/attempt/trajectory identities, the
approved fingerprint components, and the execution-store path. It does not
inherit the endpoint or credential controls and cannot select a different image
or provider. A cancellation or restrictive control mode terminates the child
within the bounded reap interval. A non-zero exit, cancellation, or worker-thread
failure is retained through the existing unknown/quarantine path before the lane
is released.

The child runtime enters the normal Runtime-v3 recovery path. Setting
`STS2_APPROVED_WORKER_FINGERPRINT=true` causes the child to use the exact
worker-approved fingerprint references rather than recomputing a potentially
different local configuration fingerprint. This does not bypass durable resume,
operation reconciliation, provider accounting, or host/gateway authority.

## Evidence and validation

The endpoint has strict parser and constant-time-comparison unit coverage. The
native process-boundary gate is in the watchdog repository and remains explicitly
ignored by default. An opt-in run must supply a separately built, digest-pinned
`sts2-harness-runtime`, `ASCENSION_WATCHDOG_REAL_HARNESS_SMOKE=1`, and a delegated
Linux cgroup-v2 environment. A passing gate confirms native watchdog-to-harness
bootstrap, peer authentication, worker probe/control, one real dispatch admission,
durable stop, and cleanup assertions. Its downstream `/bin/true` and HTTP-503
fixtures intentionally do not prove gameplay, provider settlement, deployment,
reboot, or a release.

The implementation is validated locally with the repository's strict policy,
locked offline check, warnings-denied Clippy, and all-target/all-feature tests.
The platform-specific endpoint is source-derived on non-Linux systems until an
equivalent named-pipe/ACL adapter is implemented and independently tested.
