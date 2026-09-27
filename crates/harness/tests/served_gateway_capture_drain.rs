// SPDX-License-Identifier: MIT

//! Pin that the served gateway's own streams are captured *while it runs*, not after it is
//! killed. Refs sts2-harness#559.
//!
//! # The defect under test
//!
//! `gateway_with_identity` spawns the gateway with `Stdio::piped()` on both streams, but until
//! this fix neither pipe was read until `stop` SIGKILLed the process group and then called
//! `wait_with_output()`. A pipe holds one buffer — 65,536 bytes on Linux — before its writer
//! blocks. So a gateway that wrote more than one buffer was still blocked mid-write at the
//! moment its group was killed, and every byte it had not yet written was discarded: no error,
//! no truncation notice, no diagnostic. The gateway was clipped at exactly one pipe buffer and
//! lost its own tail, which leaves a served failure holding nothing that explains why.
//!
//! Root's independent reproduction, which ran to completion before this test existed, measured
//! that wall directly: a child writing 2 MiB to a pipe with no prior reads yielded **65,536**
//! of **2,097,152** intended bytes after the group was killed — 96.9% silently lost. The
//! number is structural, not a race, which is why the assertion below is a count and not a
//! "the tail is probably there" check.
//!
//! # Why a stub gateway
//!
//! The claim is about this repository's plumbing, not about a real peer. A stub that writes a
//! known number of bytes makes the lost-byte count exact and checkable, where asserting against
//! the real gateway would pass vacuously whenever the peer happened to be quiet. It is spawned
//! through the same `gateway_with_identity` the real compositions use, so it inherits the same
//! cleared environment, the same `process_group(0)`, and the same pipes.
//!
//! The stub deliberately **writes past the pipe buffer and then keeps serving**. That is the
//! whole shape of #559: a chatty gateway that blocks mid-write while `ready` returns, serves
//! nothing for the rest of the scenario, and is then killed. A stub that wrote only a little
//! would fit in one buffer and pass on the old code, which is exactly the case the issue says
//! must fail.
//!
//! None of these tests is `#[ignore]`d, none needs an operator-built peer binary, and none
//! depends on an execution count, so they run in an ordinary `cargo test`.

#![cfg(unix)]

#[path = "support/runtime_v4_executable_composition_fixture.rs"]
// This test binary compiles the shared fixture but exercises only the capture path, so several
// `FixtureMode` variants and fixture helpers are unused here while remaining live in
// `runtime_v4_executable_composition`. The process support module carries its own crate-level
// `allow(dead_code)`, so this suppression is only needed for the fixture and is scoped to this
// binary rather than added to the shared file.
#[allow(dead_code)]
mod fixture;
#[path = "support/runtime_v4_executable_composition_process.rs"]
mod process;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};

use process::{TempDir, free_address};

use process::{MAX_CAPTURE_BYTES, MAX_TOTAL_CAPTURE_BYTES, TRUNCATION_NOTICE};

/// How many bytes the chatty stub writes to its own stderr before it serves.
///
/// Comfortably more than one pipe buffer, so the assertion is about recovering far more than
/// 64 KiB rather than about a marginal difference at the boundary.
const CHATTY_BYTES: usize = 2 * 1024 * 1024;

/// The Linux default pipe capacity. Named so the assertion can *report* the wall it is beating
/// instead of only reporting the failure.
const PIPE_BUFFER_BYTES: usize = 64 * 1024;

/// The marker written once, at the head, before the flood.
///
/// It stands in for the refusal-and-context that #548's attribution depends on, so this test
/// also proves the fix did not buy its tail by dropping the head.
const HEAD_MARKER: &str = "sts2-harness-559-chatty-gateway-head-marker";

/// A gateway that wrote far more than one pipe buffer must have far more than one pipe buffer
/// recovered.
///
/// Acceptance criterion 1. On the pre-#559 shape this recovers at most one pipe buffer, because
/// the only read of the pipe happens after the group is killed, so the left-hand value is
/// `[65,536, 65,536]` and this fails.
#[test]
fn a_chatty_gateway_has_more_than_one_pipe_buffer_recovered()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = chatty_gateway(&temporary.path)?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    let output = process::stop(gateway_process)?;
    let captured = String::from_utf8_lossy(&output.stderr);

    assert!(
        captured.len() > PIPE_BUFFER_BYTES,
        "the served capture recovered only {} bytes of the {CHATTY_BYTES} the gateway wrote, \
         which is at most one pipe buffer ({PIPE_BUFFER_BYTES}). Nothing drains the pipe while \
         the gateway runs, so a chatty gateway is clipped at one buffer and its tail is lost \
         silently (sts2-harness#559).",
        captured.len()
    );
    assert!(
        captured.len() >= CHATTY_BYTES,
        "the served capture recovered {} of the {CHATTY_BYTES} bytes the gateway wrote, so the \
         tail was still lost (sts2-harness#559)",
        captured.len()
    );
    Ok(())
}

/// The head of the stream must survive, and the tail must be identifiable as a tail.
///
/// This is the property that stops the fix being bought by truncation. #548 depends on the
/// gateway's own refusal reaching the served failure text, and #559 is about the tail that
/// arrives *after* it. Both halves are asserted here: the marker written first is still
/// present, and the bytes that followed it are all there too.
#[test]
fn a_chatty_gateway_keeps_its_head_and_its_tail() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = chatty_gateway(&temporary.path)?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    let output = process::stop(gateway_process)?;
    let captured = String::from_utf8_lossy(&output.stderr);

    assert!(
        captured.contains(HEAD_MARKER),
        "the served capture lost the head of the gateway's own stream, so #548's attribution \
         regresses into the #559 fix (sts2-harness#559)"
    );
    assert!(
        captured.contains(CHATTY_TAIL_MARKER),
        "the served capture lost the tail of the gateway's own stream: the marker written \
         after {CHATTY_BYTES} bytes of flood is absent, so the stream is still clipped \
         (sts2-harness#559)"
    );
    Ok(())
}

/// A quiet gateway's capture is unchanged by the drain.
///
/// The fix adds a background drain to every served gateway, so the ordinary case has to be
/// pinned too: a gateway that says almost nothing must still report exactly what it said.
/// Without this, "recovered more than a pipe buffer" could be satisfied by a capture that
/// returns unrelated bytes.
#[test]
fn a_quiet_gateway_still_reports_exactly_what_it_wrote() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = quiet_gateway(&temporary.path)?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    let output = process::stop(gateway_process)?;
    let captured = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        captured,
        format!("{QUIET_MARKER}\n"),
        "the served capture altered a quiet gateway's own stream"
    );
    Ok(())
}

/// The marker written *after* the flood, so the tail's arrival is a positive fact rather than
/// an inference from a byte count.
const CHATTY_TAIL_MARKER: &str = "sts2-harness-559-chatty-gateway-tail-marker";

/// A quiet gateway's only line.
const QUIET_MARKER: &str = "sts2-harness-559-quiet-gateway-marker";

/// #567's end-to-end case: a served gateway that writes past the **shared** total across **both**
/// pipes.
///
/// The unit-level tests in `gateway_capture/stream_tests.rs` drive `Stream::retain` directly,
/// which is where the accounting lives, but #567's criterion 1 asks for something narrower and
/// different: a *served gateway* that writes more than 8 MiB **across both pipes**. This stub is
/// that gateway. It is spawned through the same `gateway_with_identity` the real compositions
/// use, so it inherits the same pipes, the same `process_group(0)`, and the same shared budget.
///
/// Each pipe is flooded to *past the per-stream ceiling on its own* and past the total together.
/// That is deliberate. A stub that flooded one pipe to 5 MiB and left the other empty would never
/// touch the shared budget, so it would pass on a build whose total ceiling did not exist.
const BOTH_PIPES_BYTES: usize = 6 * 1024 * 1024;

/// The datagram `both_pipes_gateway` sends once its flood is **finished**.
///
/// #629 measured `held == 0` on this test in CI. The stub floods 12 MiB across two pipes and
/// binds its listener only afterwards, so the flood is nominally over before `process::ready`
/// can connect — but "nominally" is the whole defect. `process::free_address` binds port 0,
/// reads the port, and drops the listener, so the address is unowned by the time the stub
/// reaches `bind`. A sibling test in this same binary, running in parallel on another test
/// thread, can be handed the same port and be serving it, and then `ready` returns on a socket
/// that is not this child at all. `stop` then SIGKILLs the group while the stub is still in its
/// first iterations, and a `SIGKILL` discards writes still queued in the kernel — so the
/// capture comes back empty rather than merely short. That is the "0 bytes" in the issue title.
///
/// The sibling `chatty_gateway` is stable *because* it writes a tail marker after its flood:
/// completion is observable. This stub writes to two pipes and has no single stream to carry a
/// marker on, so the completion signal travels out of band on a datagram socket. Waiting for it
/// makes the test's precondition actually hold — a finished flood — instead of being inferred
/// from a socket that may or may not belong to this process.
///
/// The signal is a datagram rather than a file or a pipe byte because a *blocking receive* is
/// the whole point: the test must not poll, sleep, or retry, and must not burn a grace period
/// hoping bytes turned up. A datagram the stub sends is a fact the kernel delivers; the test
/// blocks on the receive and is released the instant the send happens. There is no settle time
/// to tune and nothing that could make a later measurement differ from an earlier one.
const FLOOD_DONE_SIGNAL: &[u8] = b"flooded";

/// The socket path, inside the scenario's own `TempDir`, that carries [`FLOOD_DONE_SIGNAL`].
const FLOOD_DONE_SOCKET: &str = "flood-done.sock";

/// `path` as a Python string literal, for the one line of the stub that names it.
///
/// `TempDir` builds its own paths under `std::env::temp_dir()`, so this is not a constant and
/// cannot be pasted into the script as a literal. Backslashes and quotes are escaped because
/// the value is interpolated into generated source, and a quote here would end the literal
/// rather than appear in it.
fn python_string(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let text = path.to_str().ok_or("the flood-completion path is not UTF-8")?;
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

/// Bind the end of the flood-completion handshake that the stub will send to.
///
/// Bound *before* the gateway is spawned, so the socket the stub is told to send to already
/// exists when the stub reaches its send. A datagram sent to an unbound path is discarded by
/// the kernel, so binding late would silently lose the signal and hang the test.
fn flood_done_socket(path: &Path) -> Result<UnixDatagram, Box<dyn std::error::Error>> {
    UnixDatagram::bind(path).map_err(|error| -> Box<dyn std::error::Error> {
        format!(
            "the flood-completion socket could not be bound: {error} (sts2-harness#629)"
        )
        .into()
    })
}

/// Block until the both-pipes stub reports that its flood finished.
///
/// This is one blocking `recv`. It is not a poll loop, a retry, or a timed wait: the stub sends
/// the datagram after its final `flush` on both pipes returns, so the receive returns at the
/// moment the flood is a completed fact, and the test cannot measure before that.
fn wait_for_flood(socket: &UnixDatagram) -> Result<(), Box<dyn std::error::Error>> {
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

/// #567 acceptance criterion 1, end to end: the pair the harness hands back never exceeds the
/// shared total, each cut stream announces itself, and the notice is paid for **inside** the
/// total rather than added on top of it.
///
/// The excess is checked as `held <= MAX_TOTAL_CAPTURE_BYTES` rather than `held == total - notice`
/// because the exact number depends on how the two drains interleave — whichever pipe happens to
/// be drained first spends the budget first. The load-bearing half of this assertion is the
/// `held` side: a build that charged the notice *after* the total was computed hands back
/// `total + notice`, and fails here. The unit test in `stream_tests.rs` pins the exact figure for
/// a single drain order; this one pins the bound that must hold for every one of them.
#[test]
fn a_gateway_flooding_both_pipes_is_bounded_by_the_shared_total()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = both_pipes_gateway(&temporary.path)?;
    // Bound before the spawn so the signal cannot be lost to a not-yet-existing socket.
    let flood_done = flood_done_socket(&temporary.path.join(FLOOD_DONE_SOCKET))?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    // `ready` returned, but the flood is not thereby known to have finished. Measuring now is
    // what produced `held == 0` in CI — see FLOOD_DONE_SIGNAL. Waiting for the stub to prove the
    // flood finished makes the precondition hold on every path, including the one where `ready`
    // returned against a sibling test's listener rather than this child's. The receive blocks
    // until the stub says its flood is done, so this is a fact rather than a wait.
    wait_for_flood(&flood_done)?;
    let output = process::stop(gateway_process)?;

    let stdout = &output.stdout;
    let stderr = &output.stderr;
    let held = stdout.len() + stderr.len();

    assert!(
        held <= MAX_TOTAL_CAPTURE_BYTES,
        "the served capture returned {held} bytes across both pipes, over the shared ceiling of \
         {MAX_TOTAL_CAPTURE_BYTES} (sts2-harness#567)"
    );
    assert!(
        held > MAX_CAPTURE_BYTES,
        "the served capture returned only {held} bytes across both pipes, so the shared total \
         of {MAX_TOTAL_CAPTURE_BYTES} was never reached and this test proves nothing \
         (sts2-harness#567)"
    );

    // Both pipes were flooded past the per-stream ceiling, so both carry a notice. A build that
    // announced neither is handing a reader a capture that looks whole when the pair was cut.
    for (label, stream) in [("stdout", stdout), ("stderr", stderr)] {
        assert!(
            stream
                .windows(TRUNCATION_NOTICE.len())
                .any(|window| window == TRUNCATION_NOTICE),
            "the {label} pipe was flooded to {BOTH_PIPES_BYTES} bytes, over the per-stream ceiling \
             of {MAX_CAPTURE_BYTES}, yet its capture carries no truncation notice. A reader is \
             handed a stream that looks whole when it was cut (sts2-harness#567)"
        );
    }
    Ok(())
}

/// #629's regression: a gateway whose address is served by **someone else** must still be
/// measured, not counted as zero bytes.
///
/// This is the shape the CI failure actually took. `free_address` drops its listener, so two
/// tests in this binary can be handed one port; when the other test is already serving it,
/// `process::ready` connects to *that* gateway and returns, and `stop` then SIGKILLs this
/// child's group before the stub has written anything. Because a `SIGKILL` discards writes
/// still queued in the kernel, the capture is empty rather than short — the `held == 0` in
/// #629, and the reason its assertion is worded "proves nothing".
///
/// The test holds a real listener on the address for the whole scenario, so `ready` returns
/// against a socket this child never binds. Measured on this host, that makes the kill land
/// anywhere from 0 to ~0.5 s into a flood that needs ~1 s, so without the flood-completion wait
/// the capture is empty in most runs and short in the rest — the `held == 0` in the CI failure.
/// With the wait, the flood always finishes first and the same bound holds every time. That is
/// the property being pinned: the measurement depends on the child finishing its flood, not on
/// which process happened to answer on the address.
#[test]
fn a_both_pipes_gateway_is_measured_even_when_its_address_is_already_served()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = both_pipes_gateway(&temporary.path)?;
    let flood_done = flood_done_socket(&temporary.path.join(FLOOD_DONE_SOCKET))?;
    // Stand in for the sibling test that won the port: a listener that is up before the
    // gateway is spawned and stays up until this scenario is done.
    let squatter = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = squatter.local_addr()?;
    std::thread::spawn(move || {
        for stream in squatter.incoming() {
            drop(stream);
        }
    });
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    wait_for_flood(&flood_done)?;
    let output = process::stop(gateway_process)?;

    let stdout = &output.stdout;
    let stderr = &output.stderr;
    let held = stdout.len() + stderr.len();

    assert!(
        held <= MAX_TOTAL_CAPTURE_BYTES,
        "the served capture returned {held} bytes across both pipes, over the shared ceiling of \
         {MAX_TOTAL_CAPTURE_BYTES} (sts2-harness#567)"
    );
    assert!(
        held > MAX_CAPTURE_BYTES,
        "the served capture returned only {held} bytes across both pipes even though this \
         gateway's flood completed, so the shared total of {MAX_TOTAL_CAPTURE_BYTES} was never \
         reached and this test proves nothing (sts2-harness#567, #629)"
    );
    for (label, stream) in [("stdout", stdout), ("stderr", stderr)] {
        assert!(
            stream
                .windows(TRUNCATION_NOTICE.len())
                .any(|window| window == TRUNCATION_NOTICE),
            "the {label} pipe was flooded to {BOTH_PIPES_BYTES} bytes, over the per-stream ceiling \
             of {MAX_CAPTURE_BYTES}, yet its capture carries no truncation notice \
             (sts2-harness#567)"
        );
    }
    Ok(())
}

/// The control for the test above: a gateway that stays inside the shared total carries **no**
/// notice on either pipe.
///
/// Without this, "every served gateway carries a notice" would satisfy the acceptance criterion —
/// a capture that announces truncation unconditionally is trivially bounded and useless.
#[test]
fn a_gateway_inside_the_shared_total_carries_no_truncation_notice()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = quiet_gateway(&temporary.path)?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    let output = process::stop(gateway_process)?;

    for (label, stream) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        assert!(
            !stream
                .windows(TRUNCATION_NOTICE.len())
                .any(|window| window == TRUNCATION_NOTICE),
            "the {label} pipe carried a truncation notice although the gateway stayed inside \
             every ceiling, so a reader cannot tell a cut capture from a whole one \
             (sts2-harness#567)"
        );
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

/// A stub gateway that floods its own stderr, then keeps serving until it is killed.
///
/// The flood is written in Python rather than a shell loop so the write rate is high enough to
/// fill the pipe quickly and the script is small enough to read. The `try` around the write is
/// deliberate: if the drain is *not* working, the gateway blocks mid-flood and never reaches
/// the tail marker, which is what makes this test fail loudly on the pre-#559 shape instead of
/// passing by accident.
fn chatty_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join("chatty-gateway.sh");
    fs::write(
        &path,
        format!(
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
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

/// A stub gateway that writes one line to its own stderr and then keeps serving until killed.
fn quiet_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join("quiet-gateway.sh");
    fs::write(
        &path,
        format!(
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
/// Once the flood is finished it sends [`FLOOD_DONE_SIGNAL`] on [`FLOOD_DONE_SOCKET`], which is
/// what lets the test above measure a *completed* flood. The concurrency the test exists to
/// exercise is untouched: both pipes are still written interleaved in one loop, still 6 MiB
/// each, and still drained by two independent threads racing for the one shared budget. The
/// signal is sent once, after the last of those 12 MiB, and it neither serialises the pipes nor
/// relieves either of them of contention — it only reports when the contention is over.
///
/// The send is out of band on purpose. It is *not* a byte on either pipe, so it cannot be
/// retained, truncated, or charged against the very budget the test measures — and the test
/// cannot mistake it for the child's output.
fn both_pipes_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join("both-pipes-gateway.sh");
    let done_path = python_string(&path.join(FLOOD_DONE_SOCKET))?;
    fs::write(
        &path,
        format!(
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
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}
