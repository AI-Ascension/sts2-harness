# Worker bootstrap consumer

The Linux worker entry consumes the watchdog-owned `ASC-WB01` startup frame from
one dedicated anonymous stdin pipe. The maximum JSON payload is 16384 bytes and
the whole read has one five-second deadline. It does not wait for EOF. The entry
replaces stdin with `/dev/null` and closes the owned pipe on success or failure.
Ordinary non-worker invocations and the private peer-verifier entry are unchanged.

Expected peer identity, launch nonce, component identity, and watchdog boot come
from that frame, not a worker command. `STS2_WATCHDOG_BOOT_ID` is now rejected in
worker mode, even when it matches the frame. Approved immutable runtime settings
remain environment configuration; they do not gain authority by being configured.
The harness still generates its own fresh worker boot.

The independent decoder rejects duplicate/unknown fields, malformed identities,
noncanonical integers, invalid platform paths, out-of-range creation/SID values,
oversized input, truncated frames, and buffered trailing frames. Parsing only
establishes expected policy: it does not authenticate a connected process.

`LinuxWorkerConfig::from_bootstrap` converts this validated policy to the native
listener configuration without opening any paths. The caller supplies approved
endpoint and credential references plus the expected component identifier; a
component mismatch or Windows peer is rejected. UID, GID, PID, creation token,
executable path, and digest are carried unchanged into native peer verification.
This configuration API does not itself enable the executable's serving loop.

## Linux transport identity

The native expected-peer configuration includes UID, GID, PID, process creation
identity, and approved image identity. The transport checks the connected peer
and then requires kernel-supplied credentials on each received chunk of the auth
prelude and request. It rejects a different sender, missing/truncated ancillary
data, unexpected descriptors, and a dead held pidfd. Merely passing the connected
socket to a child does not transfer the configured process identity.

After a complete request frame, the connection repeats native peer, image, and
endpoint verification under the original connection deadline before exposing
the bytes to admission. The connection borrows its listener and retains the
original socket handle, so revalidation cannot substitute a later connection.
A synthetic subprocess test replaces the authenticated sender with another
executable under the same live PID and confirms rejection before admission.

The exchange bridge selects capabilities from a local `WorkerEndpointPolicy`:
one command, read-only probe/lookup, or the watchdog owner's complete command
set. The policy is not deserialized from the request. The Linux executable uses
this bridge in its authenticated serving loop; Windows serving remains unintegrated.

`LinuxWorkerExchange::admit` joins native authentication to the worker-owned
durable admission path and consumes the original connection for its response.
Response-write status remains distinct from execution-start outcome. An admitted
reservation crosses the running fence only after a successful response write;
a failed or cancelled write retains UNKNOWN. Cancellation while the store is
busy leaves admission fenced until the uncertainty can be retained. Tests cover
native successful/failed responses and cancellation before/after polling.
This operation returns a start outcome; it does not launch gameplay itself.

The worker runtime keeps authenticated probe, lookup, terminal acknowledgment,
and restrictive control changes available after its admission fence closes.
Probes remain not-ready during pending or persisted quarantine. Dispatch and
running/resume control cannot use the recovery lease, and neither stop nor
historical lookup clears the admission fence. Capability and boot/tuple checks
still apply on these recovery paths.

The admitted runtime retains its exact handoff identity. Before a new decision,
provider reservation, operation intent, or dispatch uncertainty record, it checks
the current handoff and authenticated running control under the same store lease
as the durable operation. A changed boot, mode, sequence, or handoff closes that
execution path; changing back to running does not revive an old sequence.
Completion and uncertainty accounting for already-admitted work continue through
the separate recovery lease. Ordinary non-worker runtime handles do not acquire
this worker-specific binding.

This uses `SO_PASSCRED` and `SCM_CREDENTIALS`, whose kernel checks and privileged
exceptions are documented in [unix(7)](https://man7.org/linux/man-pages/man7/unix.7.html).
It is not a defense against privileged host compromise. The private verifier
control packet is revision 2 to carry GID; the frozen handoff JSON is unchanged.

The [copied owner artifacts](../protocol-artifact/worker-bootstrap-v1/README.md)
pin the schema and both synthetic platform fixtures to an exact owner revision.
Windows fixture decoding is supported, but the Windows native bootstrap reader
is not integrated. Linux stdin tests do not prove watchdog producer integration,
peer authentication, service installation, recovery, or gameplay execution.

Linux worker mode binds its authenticated native listener after configuration
validation and stopped boot persistence. Its owned execution thread leaves control
and historical lookup available. Accepted durable control changes cancel active
execution I/O without resetting its signal on resume or claiming effect settlement.
See [Linux executable integration](worker-local-linux.md#executable-integration)
for required endpoint settings and synthetic evidence. Windows native bootstrap
and serving, companion launcher integration, and deployment acceptance remain
separate gates. The companion launcher must deliver this frame; legacy
environment-only worker launches are incompatible.

Focused checks:

```sh
cargo test --locked --offline -p sts2-harness --lib worker_bootstrap
cargo test --locked --offline -p sts2-harness --test worker_bootstrap_linux
cargo test --locked --offline -p sts2-harness --test worker_bootstrap_entry
```

The native pipe tests cover retained writers, no-EOF completion, timeout, partial
and oversized frames, and owned-reader closure. The executable test uses synthetic
configuration and terminates before any gateway, MCP, provider, or store access.
