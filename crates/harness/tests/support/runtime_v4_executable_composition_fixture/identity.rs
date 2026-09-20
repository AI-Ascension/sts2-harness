// SPDX-License-Identifier: MIT

//! The deployment identity this synthetic downstream serves.
//!
//! A deployment does not choose its own lease. The gateway issues it from the
//! recovery store and installs it on the host, and the host is what tells the
//! runtime which fence to present. That is why the real mod validates the
//! identity on every request against the identity it was installed on and
//! answers `stale_fence` when the two differ, and why the sibling
//! `exact_restore_peer` adopts the grant of an accepted `lease_install_request`
//! into the transport it answers with.
//!
//! This fixture mirrors that: it starts on the pinned fixture identity - the
//! identity of a served deployment whose gateway kept its configured fence -
//! and moves to the grant of every install the host terminal accepts. A
//! downstream that instead echoed whatever fence arrived would answer a request
//! the gateway admitted for a deployment this process is not.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::{Value, json};

use super::{INSTANCE_ID, LEASE_EPOCH, LEASE_ID, SESSION_ID};

/// One admitted identity, as the runtime-v3 and expert envelopes carry it.
#[derive(Clone, Debug)]
pub(crate) struct Admitted {
    instance_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
}

impl Admitted {
    pub(crate) fn fields(&self) -> [(&'static str, Value); 4] {
        [
            ("instance_id", json!(self.instance_id)),
            ("session_id", json!(self.session_id)),
            ("lease_id", json!(self.lease_id)),
            ("lease_epoch", json!(self.lease_epoch)),
        ]
    }
}

pub(crate) struct Identity(Mutex<Admitted>);

impl Identity {
    /// The identity this process starts on: the pinned fixture identity, or the
    /// deployment identity an operator configured.
    ///
    /// A durable deployment is moved onto its issued lease by the install
    /// above, but a deployment whose gateway kept its configured fence has no
    /// install to learn from, so the operator has to pin the same identity here
    /// that the gateway is configured with.
    pub(crate) fn pinned() -> Self {
        let field = |name: &str, default: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| default.to_owned())
        };
        Self(Mutex::new(Admitted {
            instance_id: field("STS2_SYNTHETIC_INSTANCE_ID", INSTANCE_ID),
            session_id: field("STS2_SYNTHETIC_SESSION_ID", SESSION_ID),
            lease_id: field("STS2_SYNTHETIC_LEASE_ID", LEASE_ID),
            lease_epoch: field("STS2_SYNTHETIC_LEASE_EPOCH", &LEASE_EPOCH.to_string())
                .parse()
                .unwrap_or(LEASE_EPOCH),
        }))
    }

    /// The identity to answer with when the hop presents this deployment's
    /// fence, or `None` when it presents another one.
    ///
    /// The fence is read from the forwarded headers because that is where the
    /// gateway puts it and what it compares a response against; a caller that
    /// omits the headers therefore cannot be admitted.
    pub(crate) fn admit(&self, headers: &BTreeMap<String, String>) -> Option<Admitted> {
        let admitted = self.0.lock().ok()?.clone();
        let field = |name: &str| headers.get(name).map(String::as_str);
        let epoch = field("x-sts2-lease-epoch").and_then(|value| value.parse::<u64>().ok());
        (field("x-sts2-instance-id") == Some(admitted.instance_id.as_str())
            && field("x-sts2-session-id") == Some(admitted.session_id.as_str())
            && field("x-sts2-lease-id") == Some(admitted.lease_id.as_str())
            && epoch == Some(admitted.lease_epoch))
        .then_some(admitted)
    }

    /// Adopt the identity of a `lease_install_request` the host terminal accepted.
    ///
    /// Only an install moves the fence: a renewal carries the same lease, a
    /// revoke ends the deployment rather than moving it, and a frame the
    /// terminal refused never reaches this call.
    pub(crate) fn adopt_install(&self, raw: &[u8]) {
        let Ok(frame) = serde_json::from_slice::<Value>(raw) else {
            return;
        };
        if frame["kind"].as_str() != Some("lease_install_request") {
            return;
        }
        let grant = &frame["payload"]["grant"];
        let (Some(instance_id), Some(session_id), Some(lease_id), Some(lease_epoch)) = (
            grant["boot"]["instance_id"].as_str(),
            grant["gateway"]["session_id"].as_str(),
            grant["lease"]["lease_id"].as_str(),
            grant["lease"]["lease_epoch"].as_u64(),
        ) else {
            return;
        };
        if let Ok(mut admitted) = self.0.lock() {
            *admitted = Admitted {
                instance_id: instance_id.to_owned(),
                session_id: session_id.to_owned(),
                lease_id: lease_id.to_owned(),
                lease_epoch,
            };
        }
    }
}

/// The refusal the real mod answers a fence it is not installed on with.
pub(crate) fn stale_fence() -> Value {
    json!({"error": "stale_fence"})
}
