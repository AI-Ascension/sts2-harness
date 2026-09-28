// SPDX-License-Identifier: MIT

//! #673's regression: the gateway's own address must never be unowned, and `ready` must not be
//! able to pass against a listener the gateway never bound.
//!
//! `free_address()` used to pick the port by binding `:0`, reading it back and dropping the
//! listener, and the gateway was then told to bind that port itself. Between the drop and the
//! gateway's own `bind` the port belonged to nobody, so a test running in parallel in the same
//! binary could be handed it and be serving it. The gateway then died at its `bind` with
//! `EADDRINUSE` while `ready()` — which only requires a TCP connect to succeed — had already
//! connected to the *foreign* listener and reported the gateway up. The capture was then
//! measured as empty, which is the `held == 0` shape from #629.
//!
//! The fix is that the port is not chosen in the parent at all: the gateway binds `127.0.0.1:0`
//! and announces the address it actually got, so the port is owned continuously from allocation
//! to service. Holding the port in the parent is *not* an alternative, because a second
//! concurrent `LISTEN` on one `addr:port` is refused whatever the socket options are — verified
//! on Linux with none, `SO_REUSEADDR`, `SO_REUSEPORT`, and both.
//!
//! This test is the non-vacuity check for that claim, and it is deliberately adversarial rather
//! than a timing coincidence: a squatter thread binds ephemeral ports in a tight loop for the
//! whole scenario, which is precisely the adversary the old allocator handed a free port to. On
//! the old shape the squatter wins the gateway's port and the gateway dies at its `bind`; on the
//! new shape there is no port to win, so the gateway serves and its own bytes are recovered.
//!
//! This binary is *not* `#[ignore]`d and needs no operator-built peer binary, so it runs in an
//! ordinary `cargo test`. It is kept separate from `served_gateway_capture_drain` because it
//! asserts the allocation property rather than the capture property, and it needs a stub that
//! writes a known amount to both pipes.

#![cfg(unix)]

#[path = "support/gateway_address_announcement.rs"]
mod announcement;
#[path = "support/runtime_v4_executable_composition_fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "support/runtime_v4_executable_composition_process.rs"]
mod process;
#[path = "support/served_gateway_capture_drain_stub.rs"]
#[allow(dead_code)]
mod stub;

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use announcement::announced_address;
use process::TempDir;
use stub::both_pipes_gateway;

// The stub module reads these from its parent when it builds the scripts it generates, so the
// whole set has to be resolvable even though this binary only calls `both_pipes_gateway`. The
// unused ones are not dead — the stub module references them — so they are supplied rather than
// deleted, and duplicated rather than imported so that `served_gateway_capture_drain` keeps
// ownership of the capture assertions and the two binaries cannot be made to disagree by editing
// one of them.
pub(crate) const CHATTY_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const HEAD_MARKER: &str = "sts2-harness-559-chatty-gateway-head-marker";
pub(crate) const CHATTY_TAIL_MARKER: &str = "sts2-harness-559-chatty-gateway-tail-marker";
pub(crate) const QUIET_MARKER: &str = "sts2-harness-559-quiet-gateway-marker";
pub(crate) const BOTH_PIPES_BYTES: usize = 6 * 1024 * 1024;

/// The bytes the squatter managed to bind while the scenario ran.
///
/// Asserted on so "the squatter ran" is a measured fact rather than an assumption: a squatter
/// that never managed to bind anything would make the rest of the test vacuous, and on a host
/// where the loop is starved the test should say so instead of quietly passing.
fn spawn_port_squatter() -> (Arc<AtomicBool>, Arc<AtomicU64>) {
    let stop = Arc::new(AtomicBool::new(false));
    let taken = Arc::new(AtomicU64::new(0));
    let worker_stop = Arc::clone(&stop);
    let worker_taken = Arc::clone(&taken);
    thread::spawn(move || {
        while !worker_stop.load(Ordering::Relaxed) {
            // Binding `:0` and immediately dropping is what the old allocator did, and it is
            // the only way to obtain a port a test could have been given. Any port the
            // gateway holds is refused, so these binds land on ports nobody is using.
            if let Ok(listener) = TcpListener::bind("127.0.0.1:0") {
                let _ = listener.local_addr();
                worker_taken.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    (stop, taken)
}

/// The acceptance criterion for #673: with a hostile squatter running for the whole scenario, the
/// gateway must still bind its own address, be reported ready only against *itself*, and produce
/// a capture carrying the bytes it wrote.
///
/// On the pre-fix allocator this fails in one of two ways, and both are the real defect rather
/// than a flake: either the squatter won the port and the gateway died at `bind` (surfaced here
/// as "never announced", because the fix makes the announcement the gateway's own proof that it
/// is listening), or the squatter won it and `ready` connected to the foreign listener while the
/// gateway was already dead — which is the `held == 0` capture.
#[test]
fn a_gateway_keeps_its_own_address_while_a_squatter_takes_everything_it_can()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = both_pipes_gateway(&temporary.path)?;
    let (stop, taken) = spawn_port_squatter();

    let mut gateway_process = process::gateway(
        &stub,
        // Not a port anyone can hold. The gateway binds `:0` for itself and announces the
        // result, so this value is never bound by anyone and never read by the stub. It
        // exists only because the shared spawn path takes a `SocketAddr`, and that file sits
        // at `rust_test_preferred` and is deliberately left alone.
        SocketAddr::from(([127, 0, 0, 1], 0)),
        SocketAddr::from(([127, 0, 0, 1], 0)),
    )?;

    // The announcement is written after `listen`, so reaching this line is already proof the
    // port was owned continuously rather than allocated and released.
    let address = announced_address(&stub, announcement::ANNOUNCE_TIMEOUT)?;
    // `ready` connects to the address the gateway itself announced, so there is no longer any
    // other listener it could be fooled by: if the gateway were dead, nothing would be serving
    // that address and this would time out rather than succeed against a stranger.
    process::ready(&mut gateway_process, address)?;

    // And it produced its own bytes, which is the property a squatted address destroys: an
    // empty capture is exactly what `held == 0` meant in #629.
    let output = process::stop(gateway_process)?;
    let stdout = output.stdout.len();
    let stderr = output.stderr.len();
    assert!(
        stdout > 0 && stderr > 0,
        "the gateway produced {stdout} stdout and {stderr} stderr bytes; both pipes should carry \
         the flood it wrote, so an empty capture means the measured gateway was not the one that \
         was ready (sts2-harness#673)"
    );

    stop.store(true, Ordering::Relaxed);
    assert!(
        taken.load(Ordering::Relaxed) > 0,
        "the squatter never bound a single port, so it proved nothing and this scenario is \
         vacuous (sts2-harness#673)"
    );
    // Give the squatter a moment to observe the stop flag so it does not outlive the test.
    thread::sleep(Duration::from_millis(10));
    Ok(())
}
