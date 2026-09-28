// SPDX-License-Identifier: MIT

//! #673's regression scenario: a reserved loopback port is not stealable while it is held.
//!
//! Split out of `served_gateway_capture_drain.rs` only for file size. The test itself is the
//! same one that file ran, and `served_gateway_capture_drain` still exposes it under its own
//! name, so it is discovered and run exactly once.

use std::net::TcpListener;

use super::process::{self, ModAddress};
use super::stub::quiet_gateway;
use super::{QUIET_MARKER, TempDir, reserve};

/// #673's regression: an address drawn from the pool cannot be stolen while this test holds it.
///
/// The port used to be drawn by binding `:0`, reading the port back, and dropping the listener, so
/// the port belonged to nobody for the whole of `python3` startup and `exec`. A sibling test in
/// this binary, running on another test thread, could be handed the same port and be serving it by
/// the time this scenario's child reached its own `bind`. The child's `bind` then failed with
/// `EADDRINUSE` and the stub wrote nothing, while `ready` — which only requires a
/// `TcpStream::connect` to succeed — connected to the *sibling's* listener and reported the
/// gateway ready. `stop` then reaped a child that had never written a byte, and the capture came
/// back empty. That is the `held == 0` in #629, and the reason that failure reads as a capture
/// defect rather than as the port theft it actually is.
///
/// The squatter here is that sibling, made deterministic: a real listener is placed on the address
/// before the gateway is spawned, exactly as the reproduction on the closed PR #678
/// (`codex/629b-both-pipes-flood-signal`, commit `01d2055b`) did. The technique is promoted from
/// that branch; its datagram flood-completion signal is deliberately **not** carried over,
/// because #657 already gave this binary a better completion signal in `await_bytes` and the
/// brief for #673 says not to port that design.
///
/// What makes this a test of the *allocator* rather than of the stub is the order. `reserve` draws
/// the port and holds it, and `process::gateway` only gives it up at the fork. So by the time the
/// squatter can possibly look, this test already owns the address and the squatter is refused
/// with `EADDRINUSE` — the correct outcome, because the squatter is the intruder here. Against
/// the old `free_address` the same squatter would be *up first* and the gateway would never get
/// the port, which is exactly how this test was observed to fail when the allocator was reverted
/// in a scratch copy of this tree.
pub(super) fn a_reserved_address_cannot_be_taken_by_a_concurrent_listener()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = quiet_gateway(&temporary.path)?;
    let address = reserve()?;
    let mod_address = reserve()?;

    // Stand in for the sibling that used to win the port: a listener already up on this exact
    // address, before the gateway is spawned. Under the old allocator this is a legal thing for
    // the kernel to hand out; under `reserve` it is a bind the kernel must refuse.
    let squatter_address = address.address();
    // The `Ok` value is discarded rather than bound, because a listener that *did* bind has to be
    // dropped before the assertion fires — otherwise this test would hold a second listener on
    // the very address it is complaining about. `expect_err`/`unwrap`/`panic!` are all denied by
    // this repo's clippy config, so the refusal is inspected as data and asserted in two steps:
    // that it was refused at all, and then why.
    let refusal = TcpListener::bind(squatter_address).map(|_| ());
    assert!(
        refusal.is_err(),
        "a second listener bound {squatter_address} while this test was still holding it, so the \
         port was never owned by the test and #673's window is still open (sts2-harness#673)"
    );
    assert_eq!(
        refusal.err().map(|refused| refused.kind()),
        Some(std::io::ErrorKind::AddrInUse),
        "the squatter's bind on the reserved address {squatter_address} was refused for a reason \
         other than a live listener, so the reservation is not what refused it \
         (sts2-harness#673)"
    );

    // The gateway is spawned on the address the squatter could not have, and `ready` connects to
    // the stub's own listener. If the reservation had been dropped before the spawn, the squatter
    // would be the one serving here and this would connect to a foreign socket instead.
    let mut gateway_process = process::gateway(&stub, &address, ModAddress::Reserved(mod_address))?;
    process::ready(&mut gateway_process, squatter_address)?;
    let output = process::stop(gateway_process)?;
    let captured = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        captured,
        format!("{QUIET_MARKER}\n"),
        "the capture returned the quiet gateway's marker only when the test's own child held the \
         address, so this scenario was served by the child it spawned and not by a foreign \
         listener (sts2-harness#673)"
    );
    Ok(())
}
