// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::identity::ReceiptQueryIdentity;
#[path = "coop_receipt_query_result_validation.rs"]
mod validation;
#[path = "coop_receipt_query_result_wire.rs"]
mod wire;

const DIGEST: &str = "3e3eaedb93926b26025abb09d8028491e2632896753688c1182c698fed7d3f7c";
const TOP_LEVEL: [&str; 24] = [
    "protocol_version",
    "schema_digest",
    "provenance",
    "correlation_id",
    "instance_id",
    "session_id",
    "lease_id",
    "lease_epoch",
    "kind",
    "operation_id",
    "action_kind",
    "action_fingerprint",
    "run_id",
    "location",
    "actor_id",
    "authority_id",
    "authority_epoch",
    "expected_host_generation",
    "before_host_generation",
    "participant_ids",
    "status",
    "evidence_scope",
    "receipt",
    "error_code",
];
/// Status reported by the retained receipt cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptQueryStatus {
    Accepted,
    Settled,
    Rejected,
    Unknown,
    RecoveryRequired,
}

impl ReceiptQueryStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Settled => "settled",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
            Self::RecoveryRequired => "recovery_required",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "accepted" => Some(Self::Accepted),
            "settled" => Some(Self::Settled),
            "rejected" => Some(Self::Rejected),
            "unknown" => Some(Self::Unknown),
            "recovery_required" => Some(Self::RecoveryRequired),
            _ => None,
        }
    }
}

/// The exact fields retained for a known operation outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiptQueryReceipt {
    status: ReceiptQueryStatus,
    after_host_generation: Option<u64>,
    checkpoint_id: Option<String>,
    state_digest: Option<String>,
    effect_id: Option<String>,
    effect_kind: Option<String>,
    error_code: Option<String>,
}

impl ReceiptQueryReceipt {
    #[must_use]
    pub const fn status(&self) -> ReceiptQueryStatus {
        self.status
    }

    #[must_use]
    pub const fn after_host_generation(&self) -> Option<u64> {
        self.after_host_generation
    }

    #[must_use]
    pub fn checkpoint_id(&self) -> Option<&str> {
        self.checkpoint_id.as_deref()
    }

    #[must_use]
    pub fn state_digest(&self) -> Option<&str> {
        self.state_digest.as_deref()
    }

    #[must_use]
    pub fn effect_id(&self) -> Option<&str> {
        self.effect_id.as_deref()
    }

    #[must_use]
    pub fn effect_kind(&self) -> Option<&str> {
        self.effect_kind.as_deref()
    }

    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }
}

/// A validated retained response and its repeated immutable operation identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiptQueryResult {
    correlation_id: String,
    instance_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
    identity: ReceiptQueryIdentity,
    status: ReceiptQueryStatus,
    receipt: Option<ReceiptQueryReceipt>,
    error_code: Option<String>,
}

impl ReceiptQueryResult {
    /// Parses one canonical retained-receipt response from the MCP text payload.
    pub fn from_json(
        text: &str,
        identity: &ReceiptQueryIdentity,
        correlation: &str,
        instance: &str,
        session: &str,
        lease: &str,
        epoch: u64,
    ) -> Result<Self, ReceiptQueryError> {
        let value: Value =
            serde_json::from_str(text).map_err(|_| ReceiptQueryError::InvalidResponseShape)?;
        let result = Self::from_value(
            &value,
            identity,
            correlation,
            instance,
            session,
            lease,
            epoch,
        )?;
        if wire::canonical_response(&value).as_deref() != Some(text) {
            return Err(ReceiptQueryError::InvalidResponseShape);
        }
        Ok(result)
    }

    /// Parses a response and checks every repeated field against the request context.
    pub fn from_value(
        value: &Value,
        identity: &ReceiptQueryIdentity,
        correlation: &str,
        instance: &str,
        session: &str,
        lease: &str,
        epoch: u64,
    ) -> Result<Self, ReceiptQueryError> {
        let object = value
            .as_object()
            .ok_or(ReceiptQueryError::InvalidResponseShape)?;
        if object.len() != TOP_LEVEL.len()
            || TOP_LEVEL.iter().any(|field| !object.contains_key(*field))
        {
            return Err(ReceiptQueryError::InvalidResponseShape);
        }
        if validation::text(object, "protocol_version") != Some("coop-receipt-query-v1")
            || validation::text(object, "schema_digest") != Some(DIGEST)
            || validation::text(object, "kind") != Some("receipt_query_response")
            || validation::text(object, "evidence_scope") != Some("retained_receipt")
            || validation::text(object, "correlation_id") != Some(correlation)
            || validation::text(object, "instance_id") != Some(instance)
            || validation::text(object, "session_id") != Some(session)
            || validation::text(object, "lease_id") != Some(lease)
            || validation::generation(object.get("lease_epoch"))? != epoch
        {
            return Err(ReceiptQueryError::MetadataMismatch);
        }
        validation::validate_provenance(object.get("provenance"))?;
        if ReceiptQueryIdentity::from_object(object)
            .map_err(|_| ReceiptQueryError::IdentityMismatch)?
            != *identity
        {
            return Err(ReceiptQueryError::IdentityMismatch);
        }
        let status = object
            .get("status")
            .and_then(Value::as_str)
            .and_then(ReceiptQueryStatus::parse)
            .ok_or(ReceiptQueryError::InvalidStatus)?;
        let error_code = validation::optional_text(object.get("error_code"))?;
        let receipt = validation::parse_receipt(
            object.get("receipt"),
            status,
            identity,
            error_code.as_deref(),
        )?;
        if matches!(
            status,
            ReceiptQueryStatus::Accepted | ReceiptQueryStatus::Settled
        ) && error_code.is_some()
        {
            return Err(ReceiptQueryError::ReceiptStatusMismatch);
        }
        Ok(Self {
            correlation_id: correlation.into(),
            instance_id: instance.into(),
            session_id: session.into(),
            lease_id: lease.into(),
            lease_epoch: epoch,
            identity: identity.clone(),
            status,
            receipt,
            error_code,
        })
    }

    #[must_use]
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    #[must_use]
    pub const fn lease_epoch(&self) -> u64 {
        self.lease_epoch
    }

    #[must_use]
    pub fn identity(&self) -> &ReceiptQueryIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn status(&self) -> ReceiptQueryStatus {
        self.status
    }

    #[must_use]
    pub fn receipt(&self) -> Option<&ReceiptQueryReceipt> {
        self.receipt.as_ref()
    }

    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptQueryError {
    InvalidResponseShape,
    MetadataMismatch,
    InvalidProvenance,
    IdentityMismatch,
    InvalidStatus,
    ReceiptStatusMismatch,
    InvalidReceipt,
    InvalidGeneration,
    InvalidIdentity,
    InvalidDigest,
}

impl std::fmt::Display for ReceiptQueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidResponseShape => "receipt-query response shape is invalid",
            Self::MetadataMismatch => "receipt-query response metadata is mismatched",
            Self::InvalidProvenance => "receipt-query response provenance is invalid",
            Self::IdentityMismatch => "receipt-query response identity is mismatched",
            Self::InvalidStatus => "receipt-query response status is invalid",
            Self::ReceiptStatusMismatch => "receipt-query receipt status is inconsistent",
            Self::InvalidReceipt => "receipt-query retained receipt is invalid",
            Self::InvalidGeneration => "receipt-query generation is invalid",
            Self::InvalidIdentity => "receipt-query identity value is invalid",
            Self::InvalidDigest => "receipt-query digest is invalid",
        })
    }
}

impl std::error::Error for ReceiptQueryError {}

#[cfg(test)]
#[path = "coop_receipt_query_result_tests.rs"]
mod tests;
