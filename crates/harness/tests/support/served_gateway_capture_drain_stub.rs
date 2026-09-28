// SPDX-License-Identifier: MIT

//! The gateway stubs the capture scenarios spawn, and the shared Python fragments they build.
//!
//! Split out of `served_gateway_capture_drain.rs` so the scenarios and the stub *sources* move
//! independently. The #629 fix made these stubs longer - the bind had to move ahead of the
//! flood - and that pushed the test file past the 400-line test budget, which `repo-policy
//! --strict` promotes from a warning to a failure. These are pure text construction with no
//! assertions, so they are the natural half to move; the scenarios that assert stay together.
//! Refs sts2-harness#629.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::{BOTH_PIPES_BYTES, CHATTY_BYTES, CHATTY_TAIL_MARKER, HEAD_MARKER, QUIET_MARKER};

/// The line every stub runs before handing over to Python, exporting where it will announce the
/// address it bound.
///
/// Derived from `$0` rather than passed in by the parent so the spawn path in
/// `runtime_v4_executable_composition_process.rs` needs no new argument — that file is already at
/// `rust_test_preferred`, and growing it would trip the size gate for a change that does not
/// belong there. It also means the protocol cannot be forgotten by one stub: the path comes from
/// the same `$0` the parent derives it from, so a stub and its parent cannot disagree about it.
pub(super) const ANNOUNCE_EXPORT: &str = concat!("export STS2_ANNOUNCED_PATH=\"$0.announced\"\n",);

/// Split into `BIND_LISTENER` and `SERVE_LOOP` rather than kept as one block, because the two
/// stubs now need them at different points: `BIND_LISTENER` runs **before** the flood and
/// `SERVE_LOOP` after it. Keeping one combined block would put the bind back after the flood,
/// which is the ordering that lets another process take the port. Refs sts2-harness#629.
///
/// The bind is not wrapped in a `try`. A gateway that cannot take its own address has failed
/// the scenario, and the traceback it leaves on stderr is the evidence; swallowing it would let
/// the harness go on to report a capture that says nothing.
///
/// The port is `:0` rather than the address the harness chose, and that is the whole of the
/// #673 fix. The harness used to allocate the port itself and hand it over, which left it
/// unowned between the allocation and this `bind`; a parallel test could be serving it by then,
/// and `ready` would connect to *that* listener and call the gateway up while this process died
/// here with `EADDRINUSE`. Binding `:0` makes the kernel allocate the port atomically, so it is
/// owned continuously from allocation to service and there is no window to lose. The parent
/// cannot hold a reservation instead, because a second concurrent `LISTEN` is refused whatever
/// the socket options are — including `SO_REUSEADDR` and `SO_REUSEPORT`, which only relax the
/// `TIME_WAIT` rule and never permit two live listeners on one address.
///
/// The bound address is announced to the parent by appending it to `"$0.announced"` — the path
/// is derived from the script's own location so the spawn path needs no new argument, and the
/// file is written *after* `listen`, so its appearance proves the port is live. The write is
/// atomic (write to a temporary name, then rename) because the parent polls this path and must
/// never observe a half-written address.
pub(super) const BIND_LISTENER: &str = concat!(
    "listener = socket.socket()\n",
    "listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n",
    "listener.bind((\"127.0.0.1\", 0))\n",
    "listener.listen(8)\n",
    "announced = os.environ[\"STS2_ANNOUNCED_PATH\"]\n",
    "pending = announced + \".pending\"\n",
    "with open(pending, \"w\") as handle:\n",
    "    handle.write(\"%s:%d\" % listener.getsockname()[:2])\n",
    "os.replace(pending, announced)\n",
);

/// The tail of both stubs: accept forever on the listener bound before the flood, so the
/// gateway is still serving when the scenario stops it.
///
/// Shared rather than repeated so the two stubs cannot drift into testing different things: the
/// only difference between them is what they write to their own streams, which is the variable
/// under test.
pub(super) const SERVE_LOOP: &str = concat!(
    "while True:\n",
    "    connection, _ = listener.accept()\n",
    "    connection.close()\n",
    "PY\n",
);

/// A stub gateway that floods its own stderr, then keeps serving until it is killed.
///
/// The flood is written in Python rather than a shell loop so the write rate is high enough to
/// fill the pipe quickly and the script is small enough to read. The `try` around the write is
/// deliberate: if the drain is *not* working, the gateway blocks mid-flood and never reaches
/// the tail marker, which is what makes this test fail loudly on the pre-#559 shape instead of
/// passing by accident.
pub(super) fn chatty_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join("chatty-gateway.sh");
    fs::write(
        &path,
        format!(
            concat!(
                "#!/bin/sh\n",
                "{announce}",
                "printf '%s\\n' '",
                "{head}",
                "' >&2\n",
                "exec python3 - <<'PY'\n",
                "import os, socket, sys\n",
                "{bind}",
                "total = {flood}\n",
                "stderr = sys.stderr.buffer\n",
                "block = b'x' * 8192\n",
                "written = 0\n",
                "while written < total:\n",
                "    count = min(len(block), total - written)\n",
                "    stderr.write(block[:count])\n",
                "    written += count\n",
                "stderr.flush()\n",
                "stderr.write(b'{tail}\\n')\n",
                "stderr.flush()\n",
                "{serve}",
            ),
            head = HEAD_MARKER,
            bind = BIND_LISTENER,
            flood = CHATTY_BYTES,
            tail = CHATTY_TAIL_MARKER,
            serve = SERVE_LOOP,
            announce = ANNOUNCE_EXPORT,
        ),
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

/// A stub gateway that writes one line to its own stderr and then keeps serving until killed.
pub(super) fn quiet_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join("quiet-gateway.sh");
    fs::write(
        &path,
        format!(
            concat!(
                "#!/bin/sh\n",
                "printf '%s\\n' '",
                "{marker}",
                "' >&2\n",
                "{announce}",
                "exec python3 - <<'PY'\n",
                "import os, socket, sys\n",
                "{bind}",
                "{serve}",
            ),
            marker = QUIET_MARKER,
            bind = BIND_LISTENER,
            serve = SERVE_LOOP,
            announce = ANNOUNCE_EXPORT,
        ),
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

/// A stub gateway that floods **both** its pipes past the shared total, then keeps serving.
///
/// The two pipes are flooded to the same size, each past the 4 MiB per-stream ceiling on its own,
/// so the pair has to contend for the 8 MiB total that #567 is about. Writing them in the same
/// loop — rather than filling one and then the other — keeps the two drains genuinely concurrent,
/// so which one spends the shared budget first is left to the harness rather than prescribed by
/// the stub.
///
/// The listener is taken by `BIND_LISTENER` **before** the flood, and that ordering is the fix
/// for #629 rather than a stylistic choice. `free_address` picks a port by binding `:0`, reading
/// the port back and dropping the listener, so between that drop and the stub's own `bind` the
/// port is unowned and any concurrent test in the same binary can take it. A second live
/// `LISTEN` is refused with `EADDRINUSE` even when **both** sockets set `SO_REUSEADDR`, which
/// only relaxes the `TIME_WAIT` rule for a port whose previous listener is already closed — it
/// does not permit two simultaneous listeners.
///
/// When that happened the stub died at its `bind` having written nothing at all, and `ready` —
/// which only checks that a TCP connect succeeds — connected to the *foreign* listener in about
/// a millisecond and reported the gateway ready. `stop` then reaped a process that had never
/// written a byte, and the test's own guard fired: `held == 0`, so the shared total was never
/// reached and the test proved nothing. Binding first removes the window entirely: the port is
/// held from the moment the stub starts, and the flood runs against a listener nobody else can
/// take.
///
/// Binding first is necessary but **not** sufficient, and the second half matters as much. Once
/// the listener is up, `ready` succeeds while the flood is still in flight — that is exactly what
/// binding early buys, and taking only the bind trades a rare failure for a certain one. Measured
/// on this stub, `ready` returns with about 300 KiB captured, well under the 4 MiB the test's own
/// `held > MAX_CAPTURE_BYTES` guard demands, so that guard would fire on every run.
///
/// So the test also waits, via `GatewayProcess::await_bytes`, for the drain to have read the flood's
/// full `2 * BOTH_PIPES_BYTES` off the two pipes **before** it calls `stop`. The count is of bytes
/// read, including bytes later dropped by a ceiling, which is what makes it a completion signal
/// rather than a size signal.
///
/// A completion marker on the wire was tried first and does not work here, which is worth
/// recording: each pipe is flooded to 6 MiB against a 4 MiB per-stream ceiling and an 8 MiB
/// shared one, so a marker written *after* its pipe's flood lands past the retained head and is
/// dropped before anyone can read it. Any marker placed early enough to survive would instead sit
/// *inside* the flood and prove nothing about the flood's end. Counting bytes actually read is
/// the only signal that cannot be clipped by the very ceiling under test.
pub(super) fn both_pipes_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join("both-pipes-gateway.sh");
    fs::write(
        &path,
        format!(
            concat!(
                "#!/bin/sh\n",
                "{announce}",
                "exec python3 - <<'PY'\n",
                "import os, socket, sys\n",
                "{bind}",
                "total = {flood}\n",
                "block = b'x' * 8192\n",
                "written = 0\n",
                "while written < total:\n",
                "    count = min(len(block), total - written)\n",
                "    sys.stdout.buffer.write(block[:count])\n",
                "    sys.stdout.buffer.flush()\n",
                "    sys.stderr.buffer.write(block[:count])\n",
                "    sys.stderr.buffer.flush()\n",
                "    written += count\n",
                "{serve}",
            ),
            bind = BIND_LISTENER,
            flood = BOTH_PIPES_BYTES,
            serve = SERVE_LOOP,
            announce = ANNOUNCE_EXPORT,
        ),
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}
