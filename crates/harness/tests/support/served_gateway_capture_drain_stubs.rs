// SPDX-License-Identifier: MIT

//! The stub gateway scripts `served_gateway_capture_drain` spawns, and the flood-completion
//! handshake that makes `both_pipes_gateway`'s flood a measurable fact.
//!
//! # Why a stub gateway
//!
//! The claim is about this repository's plumbing, not about a real peer. A stub that writes a
//! known number of bytes makes the lost-byte count exact and checkable, where asserting against
//! the real gateway would pass vacuously whenever the peer happened to be quiet. It is spawned
//! through the same `gateway_with_identity` the real compositions use, so it inherits the same
//! cleared environment, the same `process_group(0)`, and the same pipes.
//!
//! The stubs deliberately **write past the pipe buffer and then keep serving**. That is the whole
//! shape of #559: a chatty gateway that blocks mid-write while `ready` returns, serves nothing
//! for the rest of the scenario, and is then killed. A stub that wrote only a little would fit in
//! one buffer and pass on the old code, which is exactly the case the issue says must fail.
//!
//! None of these is `#[ignore]`d, none needs an operator-built peer binary, and none depends on an
//! execution count, so they run in an ordinary `cargo test`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};

use super::{BOTH_PIPES_BYTES, CHATTY_BYTES, CHATTY_TAIL_MARKER, HEAD_MARKER, QUIET_MARKER};

/// The datagram `both_pipes_gateway` sends once its flood is **finished**.
///
/// #629 measured `held == 0` here. `process::free_address` binds port 0, reads the port and
/// drops the listener, so the address is unowned by the time the stub reaches `bind`. A sibling
/// test on another thread can be handed the same port and be serving it; `ready` then returns
/// on a socket that is not this child, and `stop` SIGKILLs this group while the stub is still
/// in its first iterations. SIGKILL discards writes still queued in the kernel, so the capture
/// is empty rather than merely short.
///
/// `chatty_gateway` is stable *because* it writes a tail marker after its flood, so completion
/// is observable. This stub floods two pipes and has no single stream to carry a marker, so
/// completion travels out of band on a datagram and the test blocks on one `recv` — a fact the
/// kernel delivers, not a settle period, with no poll, sleep, retry or grace to tune.
pub(super) const FLOOD_DONE_SIGNAL: &[u8] = b"flooded";

/// The socket path, inside the scenario's own `TempDir`, that carries [`FLOOD_DONE_SIGNAL`].
pub(super) const FLOOD_DONE_SOCKET: &str = "flood-done.sock";

/// `path` as a Python string literal, for the one line of the stub that names it.
///
/// `TempDir` builds its own paths, so this cannot be a constant pasted into the script. Quotes
/// and backslashes are escaped because the value is interpolated into generated source, where a
/// quote would end the literal rather than appear in it.
fn python_string(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let text = path
        .to_str()
        .ok_or("the flood-completion path is not UTF-8")?;
    let mut literal = String::with_capacity(text.len() + 2);
    literal.push('\'');
    for character in text.chars() {
        if character == '\'' || character == '\\' {
            literal.push('\\');
        }
        literal.push(character);
    }
    literal.push('\'');
    Ok(literal)
}

/// Bind the receiving end of the handshake, *before* the gateway is spawned: a datagram sent to
/// an unbound path is discarded by the kernel, so binding late would lose the signal silently.
pub(super) fn flood_done_socket(path: &Path) -> Result<UnixDatagram, Box<dyn std::error::Error>> {
    UnixDatagram::bind(path).map_err(|error| -> Box<dyn std::error::Error> {
        format!("the flood-completion socket could not be bound: {error} (sts2-harness#629)").into()
    })
}

/// Block until the stub reports its flood finished. The stub sends after its final `flush` on
/// both pipes, so the receive returns when the flood is a completed fact, not before.
pub(super) fn wait_for_flood(socket: &UnixDatagram) -> Result<(), Box<dyn std::error::Error>> {
    let mut received = [0u8; 16];
    let read = socket
        .recv(&mut received)
        .map_err(|error| -> Box<dyn std::error::Error> {
            format!(
                "the flood-completion signal was never received: {error} \
                 (sts2-harness#629)"
            )
            .into()
        })?;
    if received[..read] != *FLOOD_DONE_SIGNAL {
        return Err(format!(
            "the gateway sent {read} unexpected bytes on its flood-completion socket instead of \
             reporting a finished flood (sts2-harness#629)"
        )
        .into());
    }
    Ok(())
}

/// The tail of both stubs: bind the address the harness handed them and accept forever, so the
/// gateway is still serving when the scenario stops it.
///
/// Shared rather than repeated so the two stubs cannot drift into testing different things: the
/// only difference between them is what they write to their own stderr, which is the variable
/// under test.
const SERVE_LOOP: &str = concat!(
    "addr = os.environ[\"STS2_GATEWAY_ADDR\"]\n",
    "host, _, port = addr.rpartition(\":\")\n",
    "listener = socket.socket()\n",
    "listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n",
    "listener.bind((host, int(port)))\n",
    "listener.listen(8)\n",
    "while True:\n",
    "    connection, _ = listener.accept()\n",
    "    connection.close()\n",
    "PY\n",
);

/// Write a generated gateway script and mark it executable.
fn gateway_script(
    directory: &Path,
    name: &str,
    source: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join(name);
    fs::write(&path, source)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

/// A stub gateway that floods its own stderr, then keeps serving until it is killed.
///
/// The flood is written in Python rather than a shell loop so the write rate is high enough to
/// fill the pipe quickly and the script is small enough to read. Both writes are flushed
/// explicitly, so the tail marker is only reachable once the flood has been handed to the pipe:
/// if the drain is *not* working the gateway blocks mid-flood and never reaches the marker, which
/// is what makes this test fail loudly on the pre-#559 shape instead of passing by accident.
pub(super) fn chatty_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    gateway_script(
        directory,
        "chatty-gateway.sh",
        &format!(
            concat!(
                "#!/bin/sh\n",
                "printf '%s\\n' '",
                "{head}",
                "' >&2\n",
                "exec python3 - <<'PY'\n",
                "import os, socket, sys\n",
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
            flood = CHATTY_BYTES,
            tail = CHATTY_TAIL_MARKER,
            serve = SERVE_LOOP,
        ),
    )
}

/// A stub gateway that writes one line to its own stderr and then keeps serving until killed.
pub(super) fn quiet_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    gateway_script(
        directory,
        "quiet-gateway.sh",
        &format!(
            concat!(
                "#!/bin/sh\n",
                "printf '%s\\n' '",
                "{marker}",
                "' >&2\n",
                "exec python3 - <<'PY'\n",
                "import os, socket\n",
                "{serve}",
            ),
            marker = QUIET_MARKER,
            serve = SERVE_LOOP,
        ),
    )
}

/// A stub gateway that floods **both** its pipes past the shared total, then keeps serving.
///
/// The two pipes are flooded to the same size, each past the 4 MiB per-stream ceiling on its own,
/// so the pair has to contend for the 8 MiB total that #567 is about. Writing them in the same
/// loop — rather than filling one and then the other — keeps the two drains genuinely concurrent,
/// so which one spends the shared budget first is left to the harness rather than prescribed by
/// the stub.
///
/// Once the flood is finished it sends [`FLOOD_DONE_SIGNAL`] on [`FLOOD_DONE_SOCKET`]. The
/// concurrency this test exists to exercise is untouched: both pipes are still written
/// interleaved in one loop, still 6 MiB each, still drained by two threads racing for the one
/// shared budget. The signal is sent once, after the last of those 12 MiB, and only reports when
/// the contention is over. It is out of band on purpose — not a byte on either pipe, so it cannot
/// be retained, truncated, or charged against the budget the test measures.
pub(super) fn both_pipes_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Joined onto the directory, not onto the script path: the socket is a sibling of the script,
    // and this must be the path the test binds or the datagram reaches nobody.
    let done_path = python_string(&directory.join(FLOOD_DONE_SOCKET))?;
    gateway_script(
        directory,
        "both-pipes-gateway.sh",
        &format!(
            concat!(
                "#!/bin/sh\n",
                "exec python3 - <<'PY'\n",
                "import os, socket, sys\n",
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
                "signal = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)\n",
                "signal.sendto(b'flooded', {done_path})\n",
                "signal.close()\n",
                "{serve}",
            ),
            flood = BOTH_PIPES_BYTES,
            done_path = done_path,
            serve = SERVE_LOOP,
        ),
    )
}
