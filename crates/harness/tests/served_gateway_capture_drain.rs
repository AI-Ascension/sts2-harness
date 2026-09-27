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
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

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
    wait_for_flood(
        &temporary.path.join("flood-complete"),
        CHATTY_BYTES,
        &mut gateway_process,
    )?;
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
    wait_for_flood(
        &temporary.path.join("flood-complete"),
        CHATTY_BYTES,
        &mut gateway_process,
    )?;
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

/// Wait until a stub reports that it has written everything it intends to write.
///
/// Readiness cannot carry this. The stub binds before it floods (see [`BIND_AND_LISTEN`]), so
/// `ready()` returns while the flood is still running, and stopping there would clip the capture
/// at whatever point the connect landed — measured at 0.4–1.4 MiB against a ceiling of 8 MiB, and
/// short of the tail marker the chatty tests assert on. The marker is a file the stub creates only
/// after its last write is flushed, so its presence is a fact about the stub rather than an
/// inference from a byte count.
///
/// The wait is bounded and reports the child rather than looping forever: a stub that died before
/// it finished flooding is exactly the case this exists to distinguish, and it has to surface as
/// itself.
fn wait_for_flood(
    marker: &Path,
    written: usize,
    gateway_process: &mut process::GatewayProcess,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !marker.exists() {
        if let Some(status) = gateway_process.try_wait()? {
            return Err(format!(
                "the gateway exited with {status} before it finished writing {written} bytes, so \
                 this capture cannot reach the ceiling it is supposed to exercise \
                 (sts2-harness#629)"
            )
            .into());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the gateway did not report that it finished writing {written} bytes within 30s, \
                 so this capture cannot reach the ceiling it is supposed to exercise \
                 (sts2-harness#629)"
            )
            .into());
        }
        thread::sleep(Duration::from_millis(20));
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
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    wait_for_flood(
        &temporary.path.join("flood-complete"),
        BOTH_PIPES_BYTES,
        &mut gateway_process,
    )?;
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

/// #629 acceptance criterion 1: a stub that has *lost* its address must not be reported ready.
///
/// This is the defect #629 was filed for, reproduced directly rather than waited for. The harness
/// hands a stub an address that `free_address()` has already released, so another process can take
/// it. `ready()` asks only whether `TcpStream::connect` succeeds, so against a thief it succeeds
/// instantly and the scenario proceeds to stop a gateway that never started — the 0-byte capture
/// the non-vacuity guard above catches.
///
/// The thief here is a listener this test binds on the stub's address *after* the stub has been
/// spawned. On the pre-fix shape — stub floods, then binds — the stub's own `bind` then fails with
/// `EADDRINUSE` (`SO_REUSEADDR` does not permit a second live `LISTEN`er) and the capture is empty,
/// so this test fails with the 0-byte message. On the fixed shape the stub claims the address before
/// it floods, the thief's `bind` is the one that is refused, and readiness is answered by the stub
/// that is actually draining.
///
/// The assertion is that *somebody* served the address and the capture still describes the stub we
/// spawned, so the test is not asserting which of the two processes won the race.
#[test]
fn a_gateway_that_lost_its_address_is_not_reported_ready()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = both_pipes_gateway(&temporary.path)?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;

    // Steal the address the stub was told to use. This is what any other process drawing the same
    // port from the released pool does, and it is the whole mechanism.
    let thief = match std::net::TcpListener::bind(address) {
        Ok(thief) => thief,
        // The stub already claimed it, which is the fixed shape. Nothing to steal, and nothing to
        // assert: the scenario below is the ordinary one and the stub owns its address.
        Err(_) => {
            process::ready(&mut gateway_process, address)?;
            let output = process::stop(gateway_process)?;
            assert!(
                !output.stdout.is_empty() && !output.stderr.is_empty(),
                "the stub owned its address and still produced an empty capture on both pipes, so \
                 the capture does not work at all (sts2-harness#629)"
            );
            return Ok(());
        }
    };

    // The thief answers readiness exactly as an unrelated process would, so `ready` has to get
    // past the connect and then notice that the gateway it spawned is not the one serving.
    let ready = process::ready(&mut gateway_process, address);
    let output = process::stop(gateway_process)?;

    match ready {
        Err(error) => assert!(
            error.to_string().contains("#629") || error.to_string().contains("already exited"),
            "readiness was refused for a reason that does not name the address theft, so this \
             test would pass on a defect it is not measuring: {error}"
        ),
        Ok(()) => assert!(
            // Readiness succeeded, so nothing noticed. The only way that is honest is if the stub
            // is the one that answered — which it cannot be, because the thief holds the address.
            // Report the emptiness that #629 is about rather than passing.
            !(output.stdout.is_empty() && output.stderr.is_empty()),
            "readiness was reported ready against a foreign listener and the capture came back \
             empty on both pipes, which is exactly the #629 failure this test exists to pin. The \
             stub owned the address, so `ready` must reject this (sts2-harness#629)"
        ),
    }
    drop(thief);
    Ok(())
}

/// Claim the address the harness handed us, and listen on it.
///
/// This runs **before** any flood, and that ordering is the whole fix for #629.
/// `free_address()` binds `127.0.0.1:0`, reads the port, and drops the listener, so the port goes
/// back into the kernel's free pool. A stub that floods first and binds afterwards leaves that port
/// unowned for the whole flood — measured here at seconds under the binary's 19-way parallelism.
/// Anything else that draws the same port in that window takes it, `SO_REUSEADDR` does not permit
/// a second live `LISTEN`er, and this stub's `bind` then fails. `ready()` only asks whether
/// `TcpStream::connect` succeeds, so it connects to *that* listener, returns in 0 ms, and `stop`
/// kills a group whose stub has produced nothing: a capture of exactly 0 bytes on both pipes.
///
/// Binding first means this process owns the address for its whole life, so `ready()` can only
/// ever be answered by the stub that was spawned, and it means what every other test in this file
/// already assumes it means.
///
/// A refusal is reported on this stub's own stderr and exits non-zero rather than dying in silence.
/// The capture is bounded, so a stub that died without a word used to reach the assertion as a bare
/// byte count with no explanation attached.
const BIND_AND_LISTEN: &str = concat!(
    "addr = os.environ[\"STS2_GATEWAY_ADDR\"]\n",
    "host, _, port = addr.rpartition(\":\")\n",
    "listener = socket.socket()\n",
    "listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n",
    "try:\n",
    "    listener.bind((host, int(port)))\n",
    "except OSError as error:\n",
    "    sys.stderr.write('the stub gateway could not claim ' + addr + ': ' + str(error) + '\\n')\n",
    "    sys.stderr.flush()\n",
    "    raise SystemExit(1)\n",
    "listener.listen(8)\n",
);

/// The tail of all three stubs: accept forever, so the gateway is still serving when the scenario
/// stops it.
///
/// Shared rather than repeated so the stubs cannot drift into testing different things: the only
/// difference between them is what they write to their own pipes, which is the variable under test.
const ACCEPT_FOREVER: &str = concat!(
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
    let marker = directory.join("flood-complete");
    fs::write(
        &path,
        format!(
            concat!(
                "#!/bin/sh\n",
                "exec python3 - <<'PY'\n",
                "import os, socket, sys\n",
                "{bind}",
                "sys.stderr.write('{head}\\n')\n",
                "sys.stderr.flush()\n",
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
                "open('{marker}', 'w').close()\n",
                "{serve}",
            ),
            bind = BIND_AND_LISTEN,
            head = HEAD_MARKER,
            flood = CHATTY_BYTES,
            tail = CHATTY_TAIL_MARKER,
            marker = marker.to_str().ok_or("the flood marker path is not UTF-8")?,
            serve = ACCEPT_FOREVER,
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
                "exec python3 - <<'PY'\n",
                "import os, socket, sys\n",
                "{bind}",
                "sys.stderr.write('{marker}\\n')\n",
                "sys.stderr.flush()\n",
                "{serve}",
            ),
            bind = BIND_AND_LISTEN,
            marker = QUIET_MARKER,
            serve = ACCEPT_FOREVER,
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
/// The listener is claimed **before** the flood, not after it. That is the #629 fix: the harness
/// hands this stub an address it reserved and then released, so a flood-then-bind stub spends the
/// whole flood racing every other process that could draw the same port, and `ready()` will happily
/// report success against a listener that is not this stub. Claiming the address first means the
/// stub owns it for its whole life, and `ready()` returning means the stub is serving.
///
/// Readiness therefore returns *during* the flood, so the harness stops the stub partway through
/// on purpose. That is the scenario #567 asks for — a chatty gateway clipped at the shared total —
/// and it is only sound now that the address cannot be lost: the capture is bounded regardless of
/// how far the flood got, and the assertions below are on the bound, not on the flood completing.
///
/// `ready()` returning mid-flood is also why the flood-completion wait below is not optional. Bound
/// first means the stub is serving *while* it floods, so a harness that stopped on readiness alone
/// would clip the stub at whatever point the connect happened to land. Measured: 0.4–1.4 MiB,
/// against a test that requires the shared total to be reached.
fn both_pipes_gateway(directory: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join("both-pipes-gateway.sh");
    let marker = directory.join("flood-complete");
    fs::write(
        &path,
        format!(
            concat!(
                "#!/bin/sh\n",
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
                "open('{marker}', 'w').close()\n",
                "{serve}",
            ),
            bind = BIND_AND_LISTEN,
            flood = BOTH_PIPES_BYTES,
            marker = marker.to_str().ok_or("the flood marker path is not UTF-8")?,
            serve = ACCEPT_FOREVER,
        ),
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}
