// SPDX-License-Identifier: MIT

/// Current gateway lease identity reported by the launched runtime adapter.
///
/// These fields come from the actual post-allocation runtime state. Configured
/// launch values are not evidence of the lease the gateway returned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeLeaseBinding {
    pub instance_id: String,
    pub session_id: String,
    pub run_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
}
