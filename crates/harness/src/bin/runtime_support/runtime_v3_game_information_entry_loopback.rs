// SPDX-License-Identifier: MIT

//! A loopback address that stays **owned** from allocation until the child takes it.
//! Refs sts2-harness#681, sts2-harness#701.
//!
//! # The defect this replaces
//!
//! Two allocators in this tree used to draw a port like this:
//!
//! ```text
//! TcpListener::bind("127.0.0.1:0")?.local_addr()?
//! ```
//!
//! That binds `:0`, reads the port, and **drops the listener** inside the expression. The caller is
//! handed an address nobody owns, and the kernel is free to hand the very same port to anything else
//! that asks for `:0`.
//!
//! # Why that is worse than a scheduling accident here
//!
//! Both remaining sites hand the address to a **child process** that is expected to bind and serve
//! it, and both then wait on a readiness check that only requires `TcpStream::connect` to succeed.
//! So if anything takes the port in between, the chain is:
//!
//! 1. the allocator returns port P, and P is free;
//! 2. something else binds P and listens on it;
//! 3. the child tries to bind P and is refused with `EADDRINUSE` (errno 98);
//! 4. the readiness check connects to the **foreign** listener and reports success;
//! 5. the test proceeds against a process it did not spawn, and the child it did spawn wrote nothing.
//!
//! `SO_REUSEADDR` does not prevent this. It relaxes only the `TIME_WAIT` rule for a port whose
//! previous listener is already closed, and never permits two simultaneous `LISTEN` sockets on one
//! `addr:port`. That was measured on this host under every option combination — none,
//! `SO_REUSEADDR`, `SO_REUSEPORT`, both — and every one is `EADDRINUSE`.
//!
//! # What this type guarantees
//!
//! [`ReservedAddress`] holds the `TcpListener` itself, so from the moment a port is drawn until the
//! moment it is released, no other process can be given that port or bind it. A squatter that tries
//! is refused with `EADDRINUSE`, which is the correct outcome: the reservation is the ownership, and
//! the child is about to inherit it.
//!
//! # The window that remains, stated plainly
//!
//! The reservation **must** be released before the child binds, or the child would be refused by
//! *our own* listener. The parent cannot observe the moment the child calls `bind`, so a window
//! remains between [`ReservedAddress::release`] and the child's `bind`. It is two adjacent syscalls
//! wide, where before it was stub construction plus interpreter startup plus the whole `fork`/`exec`.
//! Closing it completely would need the child to inherit the listening socket, which is a different
//! architecture. Dropping `SO_REUSEADDR` from the stub children would allow it and is tracked
//! separately as #698.
//!
//! What is gone is the accident: the old window was wide enough for an unrelated parallel test to be
//! reliably inside it, and the new one is not wide enough for a test to aim at.
//!
//! # When you do not need this at all
//!
//! If the same process is going to `bind` the address on the next statement, do not draw a port and
//! re-bind it — bind `:0` once and read `local_addr()` off the listener you already own. That has no
//! window at all, and it is what `capture_request_headers` does. This type is for the case where the
//! binder is a **different process**.

use std::cell::Cell;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};

/// A loopback address whose port is held by this process.
///
/// Constructed by [`reserve`] and consumed by [`release`](Self::release), which yields the plain
/// [`SocketAddr`] and closes the reservation in one step. Keeping those two operations separate is
/// deliberate: `release` is the only way to get the address out, so there is no way to hand a child
/// an address this process is still listening on.
pub(super) struct ReservedAddress {
    address: SocketAddr,
    /// `Cell`, not `Option`, so `release` can take the listener through a shared reference.
    ///
    /// The callers here hold reservations in locals that are also handed to config builders and
    /// environment maps as shared borrows, and a `&mut self` release would make those borrows
    /// exclusive of each other. `Cell` is standard-library interior mutability, so there is no
    /// `unsafe` here and no second borrowing discipline for a caller to get wrong.
    listener: Cell<Option<TcpListener>>,
}

// Hand-written rather than derived: `derive(Debug)` would need `Option<TcpListener>: Copy` to reach
// `impl<T: Copy + Debug> Debug for Cell<T>`, and `TcpListener` is not `Copy`. Take the listener and
// put it straight back rather than `get`, for the same reason — `Cell::get` has the same `Copy`
// bound — and because formatting a reservation must never release it.
impl std::fmt::Debug for ReservedAddress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let listener = self.listener.take();
        let held = listener.is_some();
        self.listener.set(listener);
        formatter
            .debug_struct("ReservedAddress")
            .field("address", &self.address)
            .field("held", &held)
            .finish()
    }
}

impl ReservedAddress {
    /// The reserved address, while the reservation is still held.
    ///
    /// Only for paths that must name the address *before* the child is spawned — putting it in a
    /// child's environment, a config struct, a file. It does not transfer ownership: the port is
    /// still held and no other process can take it.
    pub(super) fn address(&self) -> SocketAddr {
        self.address
    }

    /// Give up the reservation and return the address the child should bind.
    ///
    /// This closes the listener, so the port becomes free for exactly one caller, and that caller
    /// has to be the child. Call it as late as possible — immediately before `Command::spawn()` — and
    /// no earlier: every microsecond spent holding it is a microsecond of the old defect.
    ///
    /// Calling this again after the reservation is spent is a no-op that returns the same address,
    /// not an error, so a path that may spawn more than one child on one address does not have to
    /// track whether it already released.
    pub(super) fn release(&self) -> SocketAddr {
        let Some(listener) = self.listener.take() else {
            return self.address;
        };
        // Closing is what releases the port. A listener that was only ever `bind`-ed, never
        // `listen`-ed and never `accept`-ed, has nothing pending to flush, so there is no error a
        // caller could act on. `drop` states that this statement is the release rather than an
        // accident of scope.
        drop(listener);
        self.address
    }
}

/// Draw a loopback port and **keep it** until [`ReservedAddress::release`] is called.
///
/// The port comes from the kernel's `:0` allocator, which is the same allocator the defective
/// `free_loopback_address` used. The difference is entirely in what happens next: the listener is
/// returned inside the [`ReservedAddress`] rather than dropped, so the port stays bound.
pub(super) fn reserve() -> Result<ReservedAddress, String> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .map_err(|error| format!("reserve loopback port: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("reserved loopback address unavailable: {error}"))?;
    Ok(ReservedAddress {
        address,
        listener: Cell::new(Some(listener)),
    })
}

/// Bind a loopback port for a listener this same process keeps.
///
/// The right tool when the binder is *this* process: there is no second bind and no window, so a
/// reservation would be pure ceremony. The returned listener is already `LISTEN`ing and the address
/// is owned for as long as the caller holds it.
pub(super) fn bind_loopback() -> Result<(TcpListener, SocketAddr), String> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .map_err(|error| format!("bind loopback listener: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("loopback address unavailable: {error}"))?;
    Ok((listener, address))
}

/// While a reservation is held, nothing else can take the port.
///
/// This is the property the fix exists to provide. The refusal is inspected as data rather than
/// unwrapped, so a bind that fails for an unrelated reason cannot pass this test.
#[test]
fn a_reserved_address_cannot_be_taken_by_a_concurrent_listener() {
    let reservation = reserve().expect("reserve a loopback address");
    let squatted = TcpListener::bind(reservation.address());
    assert!(
        squatted.is_err(),
        "a second listener bound {} while the reservation was still held, so the port was never \
         owned and the #681/#701 window is still open",
        reservation.address()
    );
    assert_eq!(
        squatted.err().map(|refused| refused.kind()),
        Some(std::io::ErrorKind::AddrInUse),
        "the squatter's bind was refused for a reason other than our live reservation, so the \
         reservation is not what refused it"
    );
}

/// Releasing gives the port up, and releasing twice is a no-op returning the same address.
///
/// The second half matters because the entry scenario rebinds one address across two passes;
/// `release` being idempotent is what lets that work without a separate flag.
#[test]
fn release_yields_the_address_and_is_idempotent() {
    let reservation = reserve().expect("reserve a loopback address");
    let expected = reservation.address();
    let first = reservation.release();
    let second = reservation.release();
    assert_eq!(
        first, expected,
        "release must return the address it reserved"
    );
    assert_eq!(
        second, expected,
        "a second release must be a no-op, not an error"
    );
}

/// The pre-fix shape really does leave the port unowned, so the test above is not vacuous.
///
/// This is the negative control. It does exactly what `free_loopback_address` used to do — bind
/// `:0`, read the port, drop the listener — and asserts a competing listener *can* then take it. If
/// this ever stops holding, the reservation has stopped being load-bearing and the first test would
/// be passing for the wrong reason.
#[test]
fn the_pre_fix_shape_really_does_leave_the_port_unowned() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let address = listener.local_addr().expect("read the port");
    drop(listener);
    let squatter = TcpListener::bind(address);
    assert!(
        squatter.is_ok(),
        "a released port was not takeable, so this control no longer demonstrates the defect that \
         #681/#701 defends against"
    );
}

/// `bind_loopback` is for the same-process case: one bind, no window, owned throughout.
#[test]
fn bind_loopback_owns_the_address_it_hands_out() {
    let (listener, address) = bind_loopback().expect("bind a loopback listener");
    let squatted = TcpListener::bind(address);
    assert!(
        squatted.is_err(),
        "bind_loopback handed out {address} while still holding it, which is the defect it exists \
         to avoid"
    );
    drop(listener);
}
