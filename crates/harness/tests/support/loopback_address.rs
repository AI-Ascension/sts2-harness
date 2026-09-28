// SPDX-License-Identifier: MIT

//! A loopback address that stays **owned** from allocation until the child takes it.
//! Refs sts2-harness#673.
//!
//! # The defect this replaces
//!
//! Both process support modules used to draw a port like this:
//!
//! ```text
//! let listener = TcpListener::bind("127.0.0.1:0")?;
//! Ok(listener.local_addr()?)
//! ```
//!
//! That binds, reads the port, and **drops the listener** on the way out. The caller is handed an
//! address nobody owns, and the kernel is free to hand the very same port to any other process
//! that asks for `:0`. Tests in one binary run in parallel on their own threads, so the other
//! process is usually a sibling scenario in the same binary — a sibling that draws the same port
//! and is already serving it by the time this scenario's child reaches its own `bind`.
//!
//! The child then fails with `EADDRINUSE` (errno 98) and writes nothing at all. `SO_REUSEADDR`
//! does not help: it relaxes only the `TIME_WAIT` rule for a port whose *previous* listener is
//! already closed, and never permits two simultaneous `LISTEN` sockets on one port even when both
//! set it. `ready()` cannot tell the difference, because it only requires a `TcpStream::connect`
//! to succeed — and it just succeeded against the sibling's listener. The scenario then `stop`s a
//! child that never wrote a byte and reports a capture of zero, which reads as a capture defect
//! rather than the port theft that actually happened. That is the #629 symptom.
//!
//! # What this type guarantees
//!
//! [`ReservedAddress`] holds the `TcpListener` itself, so from the moment a port is drawn until
//! the moment it is released no other process — sibling, foreign, or the kernel's own `:0`
//! allocator — can be given that port or bind it. A squatter that tries is refused with
//! `EADDRINUSE`, which is the correct outcome: the reservation is the ownership, and the child
//! is about to inherit it.
//!
//! # The window that remains, stated plainly
//!
//! The reservation **must** be released before the child binds, or the child would be refused by
//! *our own* listener. The parent cannot observe the moment the child calls `bind`, so a window
//! remains between [`ReservedAddress::release`] and the child's `bind`. It is two adjacent
//! syscalls wide (our `close` and the child's `bind`) rather than the milliseconds-to-hundreds
//! the old allocator opened, which covered stub construction, `python3` startup, and the whole
//! `fork`/`exec`. Closing it completely would need the child itself to inherit the listening
//! socket, which would change what every stub under test is testing; that is a different
//! architecture, not a better version of this one.
//!
//! What *is* gone is the accident: the old window was wide enough for an unrelated parallel test
//! to be reliably inside it, and the new one is not wide enough for a test to aim at. And the
//! failure the new window can still produce is now diagnosable, because `ready()` reports a child
//! that exited at its own `bind` instead of returning on a foreign socket.

use std::cell::Cell;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};

/// A loopback address whose port is held by this process.
///
/// Constructed by [`reserve`] and consumed by [`release`](Self::release), which yields the plain
/// [`SocketAddr`] and closes the reservation in one step. Keeping those two operations separate
/// is deliberate: `release` is the only way to get the address out, so there is no way to hand a
/// child an address that this process is still listening on.
pub(crate) struct ReservedAddress {
    address: SocketAddr,
    /// `Cell`, not `Option`, so `release` can take the listener through a shared reference.
    ///
    /// Every scenario holds its reservations in locals that are also handed to `WorkflowServiceConfig`
    /// as shared borrows, and a `&mut self` release would make those two borrows exclusive of each
    /// other. `Cell` is the standard-library interior mutability that keeps `release(&self)`, so
    /// there is no `unsafe` here and no second borrowing discipline for a caller to get wrong.
    listener: Cell<Option<TcpListener>>,
}

// Hand-written rather than derived: `derive(Debug)` would need `Option<TcpListener>: Copy` to
// reach the `impl<T: Copy + Debug> Debug for Cell<T>`, and `TcpListener` is not `Copy`. The
// address is the only field with anything to say; whether the reservation is still held is
// reported as a fact rather than by dumping the socket. `take` and put it back rather than
// `get`, for the same reason — `Cell::get` has the same `Copy` bound.
impl std::fmt::Debug for ReservedAddress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Lifted out and put straight back: formatting a reservation must not release it. `Cell`
        // has no non-`Copy` peek, so `take` is the only way to see inside one, and this is the
        // single place where that matters.
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
    /// Only for the paths that have to name the address *before* the child is spawned — putting
    /// it in a child's environment, a config struct, a file. It does not transfer ownership: the
    /// port is still held and no other process can take it.
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    /// Give up the reservation and return the address the child should bind.
    ///
    /// This closes the listener, so the port becomes free for exactly one caller, and that caller
    /// has to be the child. Call it as the last statement before `Command::spawn()` and no
    /// earlier: every microsecond spent holding it is a microsecond of the old defect.
    ///
    /// Calling this again after the reservation is spent is a no-op that returns the same
    /// address, not an error. One served scenario restarts its workflow service twice on a
    /// single address — a restart, then a foreign-authenticated second instance — and each of
    /// those spawns has to give the port up. The reservation is what protects the *first* bind;
    /// a later rebind of an address a previous child has since been reaped from is the same
    /// situation the old allocator was in, and pretending otherwise would buy nothing.
    pub(crate) fn release(&self) -> SocketAddr {
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
/// The port comes from the kernel's `:0` allocator, which is the same allocator the old
/// `free_address()` used. The difference is entirely in what happens next: the listener is
/// returned inside the [`ReservedAddress`] rather than dropped, so the port stays bound.
pub(crate) fn reserve() -> Result<ReservedAddress, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
    let address = listener.local_addr()?;
    Ok(ReservedAddress {
        address,
        listener: Cell::new(Some(listener)),
    })
}

// The downstream-shaped half of this module -- `ModAddress` and `release_onto_gateway` --
// lives in `loopback_gateway_address.rs`: this file is compiled into two test binaries and
// only one of them spawns a child with a downstream. See that file.
