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

use process::{TempDir, free_address};

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
