// SPDX-License-Identifier: MIT

use super::*;

impl MemoryPolicyOwnerManagementPort for RuntimeGameInformationOwner {
    fn current(&self, bearer: Option<&str>) -> Result<Value, ManagementError> {
        let active = self
            .owner
            .inspect_active_binding(self.management_access(bearer, &self.operator_grant_id))
            .map_err(management_owner_error)?;
        let lookup_ready = match self.management_lookup_snapshot(bearer, None) {
            Ok(_) => true,
            Err(
                PolicyOwnerError::Missing
                | PolicyOwnerError::OwnerFenced
                | PolicyOwnerError::StaleReview,
            ) => false,
            Err(error) => return Err(management_owner_error(error)),
        };
        Ok(json!({
            "schema_version": "ascension.memory-policy-owner.status.v1",
            "scope": self.scope,
            "deployment_config_sha256": self.deployment_sha256,
            "active_binding": active,
            "lookup_ready": lookup_ready,
        }))
    }

    fn inspect_policy(
        &self,
        bearer: Option<&str>,
        reference: &SavedPolicyRef,
    ) -> Result<String, ManagementError> {
        let saved: SavedPolicy = self
            .owner
            .inspect_policy(
                self.management_access(bearer, &self.operator_grant_id),
                reference,
            )
            .map_err(management_owner_error)?;
        Ok(hex_encode(saved.raw_bytes()))
    }

    fn inspect_review(
        &self,
        bearer: Option<&str>,
        review_id: &str,
    ) -> Result<Value, ManagementError> {
        let review = self
            .owner
            .inspect_review(
                self.management_access(bearer, &self.operator_grant_id),
                review_id,
            )
            .map_err(management_owner_error)?;
        serde_json::to_value(review).map_err(|_| {
            ManagementError::store(
                "memory_policy_review_encode",
                "selected-memory policy review could not be encoded",
            )
        })
    }

    fn execute(
        &self,
        bearer: Option<&str>,
        command: PolicyCommand,
    ) -> Result<Value, ManagementError> {
        let receipt = self
            .owner
            .execute(
                self.management_access(bearer, &self.operator_grant_id),
                command,
            )
            .map_err(management_owner_error)?;
        serde_json::to_value(receipt).map_err(|_| {
            ManagementError::store(
                "memory_policy_receipt_encode",
                "selected-memory policy receipt could not be encoded",
            )
        })
    }
}

fn management_owner_error(error: PolicyOwnerError) -> ManagementError {
    match error {
        PolicyOwnerError::Unauthenticated => {
            ManagementError::authentication("owner_unauthenticated", "owner authentication failed")
        }
        PolicyOwnerError::PermissionDenied | PolicyOwnerError::GrantRevoked => {
            ManagementError::forbidden("owner_permission_denied", "owner grant denied the request")
        }
        PolicyOwnerError::ScopeMismatch => {
            ManagementError::forbidden("owner_scope_mismatch", "owner grant scope does not match")
        }
        PolicyOwnerError::StaleReview
        | PolicyOwnerError::OwnerFenced
        | PolicyOwnerError::Conflict => {
            ManagementError::conflict("owner_fenced", "owner policy or review is stale")
        }
        PolicyOwnerError::Missing => {
            ManagementError::invalid("owner_record_missing", "owner record was not found")
        }
        PolicyOwnerError::Capacity => {
            ManagementError::budget("owner_capacity", "owner record exceeds its bound")
        }
        PolicyOwnerError::SchemaInvalid | PolicyOwnerError::UnsupportedNumericRepresentation => {
            ManagementError::invalid("owner_schema_invalid", "owner command is invalid")
        }
        PolicyOwnerError::StoreIncompatible
        | PolicyOwnerError::Corrupt
        | PolicyOwnerError::Unavailable
        | PolicyOwnerError::LostReply
        | PolicyOwnerError::PersistenceFailure
        | PolicyOwnerError::Memory(_) => ManagementError::store(
            "owner_store_failure",
            "selected-memory policy owner could not complete the request",
        ),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[(byte >> 4) as usize]));
        output.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    output
}
