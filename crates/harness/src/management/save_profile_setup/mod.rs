// SPDX-License-Identifier: MIT

//! Typed, capability-gated mapping of authored save-profile setup operations.
//!
//! Issue #102 requires the harness to drive save-profile discovery, selection
//! and disposable provisioning through owner descriptors and the accepted MCP
//! routes, without acquiring game, filesystem or process authority itself. This
//! module owns only the *contract* for that mapping: which operation maps to
//! which single accepted tool and route, which permission it requires, and which
//! baseline and active-run preconditions must hold before a mutation is even
//! attempted.
//!
//! Two rules are structural rather than advisory. First, discovery is
//! effect-free: a read operation that names a profile or a baseline fence is
//! refused, so discovery can never be a disguised selection. Second, a
//! disposable provision may not require a baseline fence, because that baseline
//! does not exist until the gateway allocates it; the authoritative readback is
//! what later admits the profile to a selection. No route is assembled at
//! runtime from supplied text.

mod error;
mod operation;
mod request;
mod setup;

pub use error::ProfileSetupError;
pub use operation::{
    PROFILE_ROUTE_CONTRACT, PROFILE_ROUTE_REVISION, ProfileGrant, ProfileSetupOperation,
};
pub use request::{
    MAX_PROFILE_ID_BYTES, PROFILE_SETUP_SCHEMA_VERSION, ProfileBaselineFence, ProfileSetupGrants,
    ProfileSetupOperationDocument, ProfileSetupRequest, is_instance_identity, is_profile_identity,
};
pub use setup::{
    AdmittedProfileSetup, ProfileReadback, VerifiedProfileReadback, admit_profile_setup,
};
