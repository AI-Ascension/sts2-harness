// SPDX-License-Identifier: MIT

//! Opt-in persistent-provider session contracts and broker policy.
//!
//! This module is deliberately additive to the stateless Exo/Ollama paths.  It owns only
//! application-side identities, lifecycle fencing and a narrow native line protocol; it never
//! grants a provider game authority or exposes a raw RPC surface.

mod broker;
mod protocol;
mod transport;
mod types;

pub use broker::{BrokerSnapshot, ProviderSessionBroker};
pub use protocol::{
    NativeFrame, NativeFrameKind, NativePeerError, NativeResponse, parse_native_frame,
    parse_native_request,
};
pub use transport::{NativeTransportError, OwnedNativeTransport};
pub use types::*;
