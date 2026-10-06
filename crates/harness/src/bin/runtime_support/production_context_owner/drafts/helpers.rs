// SPDX-License-Identifier: MIT

fn validate_create_request(
    request: &ContextOwnerDraftCreateRequest,
) -> Result<(), ManagementError> {
    if request.schema_version != CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION {
        return Err(ManagementError::invalid(
            "context_draft_request_schema",
            "unsupported draft request schema",
        ));
    }
    validate_identifier("context_request_id", &request.request_id)?;
    validate_identifier("context_draft_id", &request.draft_id)?;
    validate_identifier("context_revision_id", &request.base_revision_id)?;
    Ok(())
}

fn validate_patch_request(request: &ContextOwnerDraftPatchRequest) -> Result<(), ManagementError> {
    if request.schema_version != CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION
        || request.operations.is_empty()
        || request.operations.len() > sts2_harness::management::MAX_CONTEXT_OWNER_DRAFT_OPERATIONS
    {
        return Err(ManagementError::invalid(
            "context_draft_patch_schema",
            "draft patch is empty or outside its versioned bound",
        ));
    }
    validate_identifier("context_request_id", &request.request_id)?;
    validate_identifier("context_draft_id", &request.draft_id)?;
    Ok(())
}

fn validate_preview_request(request: &ContextOwnerPreviewRequest) -> Result<(), ManagementError> {
    if request.schema_version != CONTEXT_OWNER_PREVIEW_REQUEST_SCHEMA_VERSION {
        return Err(ManagementError::invalid(
            "context_preview_request_schema",
            "unsupported preview request schema",
        ));
    }
    validate_identifier("context_request_id", &request.request_id)?;
    validate_identifier("context_draft_id", &request.draft_id)?;
    Ok(())
}

fn validate_mutation_lookup(
    request: &ContextOwnerMutationRequest,
) -> Result<(&str, String), ManagementError> {
    let (request_id, payload_digest) = match request {
        ContextOwnerMutationRequest::CreateDraft(request) => {
            validate_create_request(request)?;
            (&request.request_id, request_digest(request)?)
        }
        ContextOwnerMutationRequest::PatchDraft(request) => {
            validate_patch_request(request)?;
            (&request.request_id, request_digest(request)?)
        }
        ContextOwnerMutationRequest::CreatePreview(request) => {
            validate_preview_request(request)?;
            (&request.request_id, request_digest(request)?)
        }
    };
    validate_identifier("context_request_id", request_id)?;
    Ok((request_id, payload_digest))
}

fn request_digest<T: Serialize>(request: &T) -> Result<String, ManagementError> {
    let bytes = serde_json::to_vec(request).map_err(|error| {
        ManagementError::invalid("context_owner_request_encode", error.to_string())
    })?;
    Ok(sts2_harness::sha256_hex(bytes))
}

fn item_key(reference: &ContextItemRef) -> String {
    format!("{}:{}", reference.item_id, reference.version)
}

fn insert_eligible(
    registry: &mut BTreeMap<String, EligibleItem>,
    item: ContextItem,
    source: SourceIdentity,
) -> Result<(), ManagementError> {
    let key = item_key(&item.reference);
    if let Some(existing) = registry.get(&key) {
        if existing.item != item {
            return Err(ManagementError::conflict(
                "context_owner_item_identity_conflict",
                "owner-advertised sources contain conflicting bytes for one item reference",
            ));
        }
        return Ok(());
    }
    registry.insert(key, EligibleItem { item, source });
    Ok(())
}

fn state_corrupt() -> ManagementError {
    ManagementError::unavailable(
        "context_owner_state_corrupt",
        "encrypted owner draft state is invalid",
    )
}

fn store_error(error: sts2_harness::context_control::DurableControlStoreError) -> ManagementError {
    match error {
        sts2_harness::context_control::DurableControlStoreError::OwnerContextConflict => {
            ManagementError::conflict(
                "context_owner_draft_conflict",
                "another owner operation committed a newer durable draft state",
            )
        }
        sts2_harness::context_control::DurableControlStoreError::TooLarge => {
            ManagementError::unavailable(
                "context_owner_state_too_large",
                "owner context state exceeded its encrypted storage bound",
            )
        }
        _ => ManagementError::unavailable("context_owner_state_store", error.to_string()),
    }
}

pub(super) fn owner_lock_error() -> ManagementError {
    ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
}

pub(super) fn owner_unavailable() -> ManagementError {
    ManagementError::unavailable(
        "context_owner_draft_unavailable",
        "current owner observation is unavailable",
    )
}

fn owner_capacity() -> ManagementError {
    ManagementError::unavailable(
        "context_owner_capacity",
        "owner draft state reached its bounded capacity",
    )
}

fn stale_boundary() -> ManagementError {
    ManagementError::conflict(
        "context_owner_boundary_stale",
        "request does not name the exact current owner boundary",
    )
}

fn item_unavailable() -> ManagementError {
    ManagementError::invalid(
        "context_owner_item_unavailable",
        "item is not in the current owner-advertised eligible registry",
    )
}
