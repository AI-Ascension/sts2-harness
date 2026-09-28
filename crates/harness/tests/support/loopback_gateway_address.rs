// SPDX-License-Identifier: MIT

//! The gateway and downstream addresses a served child is handed at spawn.
//!
//! Split out from `loopback_address.rs` because that file is compiled into **two** test binaries
//! via `#[path]`, and only the executable-composition one spawns a child with a downstream. The
//! REST composition support holds a gateway reservation and a live `ModServer` but never gives a
//! port up at a spawn, so `ModAddress` and `release_onto_gateway` were dead code there and
//! `-D warnings` turned that into a build failure. Refs sts2-harness#673.

use super::loopback_address::ReservedAddress;
use std::net::SocketAddr;

/// A downstream address, in the two forms a test actually holds one in.
///
/// A live `ModServer` is a *real* listener that keeps serving for the whole scenario, so the
/// composition tests hand over an address this test genuinely already owns and no reservation is
/// involved. The capture scenarios have no downstream at all — their stub never dials it — so
/// theirs is a [`ReservedAddress`] held purely to stop a parallel scenario drawing that port.
/// [`ModAddress::release`] makes the two indistinguishable at the spawn, which is the point: the
/// child is told a port nothing else in the binary is holding, either because this test held it
/// until now or because this test never let go of it.
#[derive(Debug)]
pub(crate) enum ModAddress {
    /// An address a live `ModServer` in this test is already serving.
    Live(SocketAddr),
    /// An address this test reserved and is giving up at the spawn.
    Reserved(ReservedAddress),
}

impl ModAddress {
    /// The address to hand the child, releasing the reservation if this is one.
    ///
    /// Call it as the last statement before `Command::spawn()`, for the same reason
    /// [`ReservedAddress::release`] has to be there.
    pub(crate) fn release(&self) -> SocketAddr {
        match self {
            ModAddress::Live(address) => *address,
            ModAddress::Reserved(reservation) => reservation.release(),
        }
    }
}

/// Name the gateway and downstream addresses on `command`, releasing both reservations first.
///
/// This is the one place the release happens, so it is also the one place a caller can check that
/// a scenario did it. Call it last before the spawn: the child owns the `bind`, and the parent
/// cannot observe it, so the gap between this and the child's own `bind` is what remains of #673.
pub(crate) fn release_onto_gateway(
    command: &mut std::process::Command,
    gateway: &ReservedAddress,
    mod_address: &ModAddress,
) {
    command.env("STS2_GATEWAY_ADDR", gateway.release().to_string());
    command.env("STS2_MOD_ADDR", mod_address.release().to_string());
}
