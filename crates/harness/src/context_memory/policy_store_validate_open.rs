// SPDX-License-Identifier: MIT

fn validate_open(key: &[u8; 32], consent: &PolicyStoreConsent) -> Result<(), PolicyOwnerError> {
    if key.iter().all(|byte| *byte == 0) {
        return Err(PolicyOwnerError::PermissionDenied);
    }
    if let PolicyStoreConsent::ApprovedPrivate { policy_ref } = consent
        && !valid_id(policy_ref)
    {
        return Err(PolicyOwnerError::PermissionDenied);
    }
    Ok(())
}
