// SPDX-License-Identifier: MIT

//! The host half of the pinned `watchdog-host-lease-control-v1` sideband.
//!
//! `sts2-gateway` drives every recovery mutation to the mod address over the
//! fixed `POST /api/v1/runtime/recovery` mux, so whatever terminates that hop
//! has to answer the fence and lease-control frames itself. The gateway's own
//! tests terminate it with a fake that lives inside the gateway test binary;
//! this module is the same terminal for the harness's long-lived synthetic
//! downstream, which is the only downstream a soak campaign runs.
//!
//! The terminal is deliberately narrow. It answers `host_fence_request` and the
//! three lease-control requests, verifies the request proof before it produces
//! any acknowledgment, copies every identity out of the frame the gateway sent
//! instead of synthesizing one, and refuses anything else by name.

use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::host_lease_control_canonical::{
    HOST_LEASE_CONTROL_CONTRACT, HOST_LEASE_CONTROL_SCHEMA_DIGEST, RECOVERY_CONTRACT,
    RECOVERY_SCHEMA_DIGEST, decode_host_lease_key, parse_canonical_input, proof_for, timestamp,
    verify_frame_proof,
};

/// The largest frame the pinned profile admits on the recovery mux.
const MAX_FRAME_BYTES: usize = 262_144;
/// The host principal the gateway is configured to trust.
const DEFAULT_HOST_PRINCIPAL: &str = "00000000-0000-4000-8000-00000000000e";
/// The fence request carries the recovery envelope, not the lease envelope.
const FENCE_REQUEST: &str = "host_fence_request";
const FENCE_CAPABILITY: &str = "host_fence";
const FENCE_ACCEPTED: &str = "FENCE_ACCEPTED";

/// The fixed proof-domain table. A caller never selects a domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostLeaseKind {
    Install,
    Renew,
    Revoke,
}

impl HostLeaseKind {
    pub(crate) fn from_request_name(name: &str) -> Option<Self> {
        match name {
            "lease_install_request" => Some(Self::Install),
            "lease_renew_request" => Some(Self::Renew),
            "lease_revoke_request" => Some(Self::Revoke),
            _ => None,
        }
    }

    const fn response_name(self) -> &'static str {
        match self {
            Self::Install => "lease_install_response",
            Self::Renew => "lease_renew_response",
            Self::Revoke => "lease_revoke_response",
        }
    }

    const fn capability(self) -> &'static str {
        match self {
            Self::Install => "lease_install",
            Self::Renew => "lease_renew",
            Self::Revoke => "lease_revoke",
        }
    }

    pub(crate) const fn request_domain(self) -> &'static str {
        match self {
            Self::Install => "host-lease-control/v1/lease-install-request",
            Self::Renew => "host-lease-control/v1/lease-renew-request",
            Self::Revoke => "host-lease-control/v1/lease-revoke-request",
        }
    }

    pub(crate) const fn ack_domain(self) -> &'static str {
        match self {
            Self::Install => "host-lease-control/v1/lease-install-ack",
            Self::Renew => "host-lease-control/v1/lease-renew-ack",
            Self::Revoke => "host-lease-control/v1/lease-revoke-ack",
        }
    }

    const fn status(self) -> &'static str {
        match self {
            Self::Install => "INSTALLED",
            Self::Renew => "RENEWED",
            Self::Revoke => "REVOKED",
        }
    }
}

/// The signed host terminal, configured from the campaign environment.
pub(crate) struct HostLeaseControl {
    key: [u8; 32],
    principal_id: String,
}

impl HostLeaseControl {
    /// The terminal for an in-process caller that supplies the pinned key
    /// directly, which is how the conformance target avoids touching the
    /// process environment.
    #[allow(
        dead_code,
        reason = "used by the host lease-control conformance target"
    )]
    pub(crate) fn new(key: [u8; 32], principal_id: &str) -> Self {
        Self {
            key,
            principal_id: principal_id.to_owned(),
        }
    }

    /// Reads the host sideband configuration, or `None` when the operator has
    /// not enabled it. An enabled terminal with an invalid key is an error
    /// rather than a silent downgrade.
    pub(crate) fn from_env() -> Result<Option<Self>, String> {
        let Ok(encoded) = std::env::var("STS2_SYNTHETIC_HOST_LEASE_KEY") else {
            return Ok(None);
        };
        let key = decode_host_lease_key(&encoded)?;
        let principal_id = std::env::var("STS2_SYNTHETIC_HOST_PRINCIPAL_ID")
            .unwrap_or_else(|_| String::from(DEFAULT_HOST_PRINCIPAL));
        Ok(Some(Self { key, principal_id }))
    }

    /// Terminates one recovery-mux frame.
    ///
    /// Order matters and follows the profile: the raw bytes are validated as a
    /// canonical input, then the envelope, then the request proof, and only
    /// then is an acknowledgment produced.
    pub(crate) fn respond(&self, raw: &[u8]) -> Result<Value, String> {
        let frame = parse_canonical_input(raw, MAX_FRAME_BYTES)?;
        let kind = required_str(&frame, "/kind")?;
        if kind == FENCE_REQUEST {
            return self.fence_response(&frame);
        }
        let kind = HostLeaseKind::from_request_name(kind)
            .ok_or_else(|| format!("the host refuses the unhandled frame kind {kind}"))?;
        require_envelope(
            &frame,
            HOST_LEASE_CONTROL_CONTRACT,
            HOST_LEASE_CONTROL_SCHEMA_DIGEST,
        )?;
        verify_frame_proof(&frame, kind.request_domain(), &self.key)?;
        self.lease_ack(&frame, kind)
    }

    /// The fence acknowledgment. Every identity is copied from the boot the
    /// gateway presented, which is what makes the fence comparison accept it.
    fn fence_response(&self, frame: &Value) -> Result<Value, String> {
        require_envelope(frame, RECOVERY_CONTRACT, RECOVERY_SCHEMA_DIGEST)?;
        let boot = required(frame, "/payload/boot")?;
        let mut fence = Map::new();
        fence.insert(
            String::from("host_fence_id"),
            json!(Uuid::new_v4().to_string()),
        );
        for name in [
            "deployment_id",
            "instance_id",
            "instance_incarnation",
            "boot_id",
        ] {
            fence.insert(
                String::from(name),
                json!(required_str(boot, &format!("/{name}"))?),
            );
        }
        fence.insert(
            String::from("authority_generation"),
            json!(required_u64(boot, "/authority_generation")?),
        );
        fence.insert(String::from("fence_generation"), json!(1));
        fence.insert(String::from("created_at"), json!(timestamp()));
        Ok(self.envelope(
            frame,
            RECOVERY_CONTRACT,
            RECOVERY_SCHEMA_DIGEST,
            timestamp(),
            FENCE_CAPABILITY,
            json!({
                "kind": "host_fence_response",
                "payload": {
                    "result": {
                        "status": FENCE_ACCEPTED,
                        "retryable": false,
                        "retry_after_seconds": Value::Null,
                    },
                    "fence": Value::Object(fence),
                },
            }),
        ))
    }

    /// The signed lease-control acknowledgment.
    ///
    /// An install or renewal binds the acknowledgment to the grant's expiry; a
    /// revoke reports no expiry at all, which is what each acknowledgment
    /// validator requires.
    fn lease_ack(&self, frame: &Value, kind: HostLeaseKind) -> Result<Value, String> {
        let sent_at = required_str(frame, "/sent_at")?.to_owned();
        let recorded_at = sent_at.clone();
        let grant = required(frame, "/payload/grant")?;
        let lease = required(grant, "/lease")?;
        let fence = required(grant, "/fence")?;
        let boot = required(grant, "/boot")?;
        let expires_at = match kind {
            HostLeaseKind::Revoke => Value::Null,
            HostLeaseKind::Install | HostLeaseKind::Renew => {
                required(lease, "/expires_at")?.clone()
            }
        };
        let renew_sequence = match kind {
            HostLeaseKind::Renew => required(frame, "/payload/renew_sequence")?.clone(),
            HostLeaseKind::Install | HostLeaseKind::Revoke => Value::Null,
        };
        let mut ack = self.envelope(
            frame,
            HOST_LEASE_CONTROL_CONTRACT,
            HOST_LEASE_CONTROL_SCHEMA_DIGEST,
            sent_at,
            kind.capability(),
            json!({
                "kind": kind.response_name(),
                "payload": {
                    "ack": {
                        "result": {
                            "status": kind.status(),
                            "retryable": false,
                            "retry_after_seconds": Value::Null,
                        },
                        "installation_id": required(frame, "/payload/installation_id")?,
                        "grant_digest": required(frame, "/payload/grant_digest")?,
                        "boot_id": required_str(boot, "/boot_id")?,
                        "instance_incarnation": required_str(boot, "/instance_incarnation")?,
                        "host_fence_id": required_str(fence, "/host_fence_id")?,
                        "fence_generation": required_u64(fence, "/fence_generation")?,
                        "lease_id": required_str(lease, "/lease_id")?,
                        "lease_epoch": required_u64(lease, "/lease_epoch")?,
                        "host_install_generation": 1,
                        "recorded_at": recorded_at,
                        "renew_sequence": renew_sequence,
                        "expires_at": expires_at,
                    },
                },
            }),
        );
        let proof = proof_for(&ack, kind.ack_domain(), &self.key)?;
        if let Some(auth) = ack.get_mut("auth").and_then(Value::as_object_mut) {
            auth.insert(String::from("proof"), json!(proof));
        }
        Ok(ack)
    }

    fn envelope(
        &self,
        frame: &Value,
        contract: &str,
        schema_digest: &str,
        sent_at: String,
        capability: &str,
        body: Value,
    ) -> Value {
        let mut envelope = json!({
            "contract": contract,
            "schema_digest": schema_digest,
            "message_id": Uuid::new_v4().to_string(),
            "correlation_id": frame.get("correlation_id").cloned().unwrap_or(Value::Null),
            "sent_at": sent_at,
            "actor": {"principal_id": self.principal_id, "role": "host"},
            "auth": {
                "principal_id": self.principal_id,
                "capability": capability,
                "proof": Value::Null,
            },
        });
        if let (Some(envelope), Value::Object(body)) = (envelope.as_object_mut(), body) {
            for (name, value) in body {
                envelope.insert(name, value);
            }
        }
        envelope
    }
}

fn require_envelope(frame: &Value, contract: &str, digest: &str) -> Result<(), String> {
    if required_str(frame, "/contract")? != contract {
        return Err(format!("the frame is not a {contract} frame"));
    }
    if required_str(frame, "/schema_digest")? != digest {
        return Err(String::from("the frame presents a different schema digest"));
    }
    Ok(())
}

fn required<'a>(value: &'a Value, pointer: &str) -> Result<&'a Value, String> {
    value
        .pointer(pointer)
        .ok_or_else(|| format!("the frame omits {pointer}"))
}

fn required_str<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, String> {
    required(value, pointer)?
        .as_str()
        .ok_or_else(|| format!("the frame omits a string {pointer}"))
}

fn required_u64(value: &Value, pointer: &str) -> Result<u64, String> {
    required(value, pointer)?
        .as_u64()
        .ok_or_else(|| format!("the frame omits an integer {pointer}"))
}
