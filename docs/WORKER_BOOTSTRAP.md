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

## Linux transport identity

The native expected-peer configuration includes UID, GID, PID, process creation
identity, and approved image identity. The transport checks the connected peer
and then requires kernel-supplied credentials on each received chunk of the auth
prelude and request. It rejects a different sender, missing/truncated ancillary
data, unexpected descriptors, and a dead held pidfd. Merely passing the connected
socket to a child does not transfer the configured process identity.

This uses `SO_PASSCRED` and `SCM_CREDENTIALS`, whose kernel checks and privileged
exceptions are documented in [unix(7)](https://man7.org/linux/man-pages/man7/unix.7.html).
It is not a defense against privileged host compromise. The private verifier
control packet is revision 2 to carry GID; the frozen handoff JSON is unchanged.

The [copied owner artifacts](../protocol-artifact/worker-bootstrap-v1/README.md)
pin the schema and both synthetic platform fixtures to an exact owner revision.
Windows fixture decoding is supported, but the Windows native bootstrap reader
is not integrated. Linux stdin tests do not prove watchdog producer integration,
peer authentication, service installation, recovery, or gameplay execution.

Worker mode still stops with the fixed missing-listener error after successful
configuration and boot persistence. It must not be deployed as an operational
worker until the native listener, peer policy, and execution-loop wiring pass
their separate integration gates. The companion launcher must deliver this frame;
legacy environment-only worker launches are incompatible.

Focused checks:

```sh
cargo test --locked --offline -p sts2-harness --lib worker_bootstrap
cargo test --locked --offline -p sts2-harness --test worker_bootstrap_linux
cargo test --locked --offline -p sts2-harness --test worker_bootstrap_entry
```

The native pipe tests cover retained writers, no-EOF completion, timeout, partial
and oversized frames, and owned-reader closure. The executable test uses synthetic
configuration and terminates before any gateway, MCP, provider, or store access.
