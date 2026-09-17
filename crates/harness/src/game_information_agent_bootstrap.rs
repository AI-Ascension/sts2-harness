// SPDX-License-Identifier: MIT

fn handle_bootstrap<M: LookupMcpPort>(
    session: &mut LookupSession,
    mcp: &mut M,
    operation_id: String,
    request: Vec<u8>,
) -> LookupFeedback {
    let request = match validation::decode_strict(&request) {
        Ok(request) => request,
        Err(error) => return LookupFeedback::Error(error.into()),
    };
    let mut request_shape = request.clone();
    request_shape["scope"] = serde_json::json!({
        "instance_id": "pending",
        "run_id": session.binding.scope.run_id,
        "authority_epoch": session.binding.authority_epoch,
        "content_manifest_id": session.binding.content_manifest_id,
        "locale": session.binding.locale
    });
    if let Err(error) =
        crate::game_information_binding::game_information_bootstrap::validate_request(
            &request_shape,
        )
    {
        let error = bootstrap_error(error);
        session.record_bootstrap_error(&operation_id, request, error.clone());
        return LookupFeedback::Error(error);
    }
    match mcp.call_live_observation_bootstrap(&request) {
        Ok(raw) => {
            let response = match validation::decode_strict(&raw) {
                Ok(response) => response,
                Err(error) => {
                    session.record_bootstrap_error(&operation_id, request, error.into());
                    return LookupFeedback::Error(LookupError::Invalid);
                }
            };
            let mut validated_request = request.clone();
            validated_request["scope"] = response["scope"].clone();
            // The provider sends a transport-neutral `pending` correlation.
            // The owner replaces it with the authenticated MCP correlation
            // before validating and recording the response.
            validated_request["correlation_id"] = response["correlation_id"].clone();
            let snapshot = match crate::game_information_binding::game_information_bootstrap::select_snapshot(
                &validated_request,
                &response,
            ) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    let error = bootstrap_error(error);
                    session.record_bootstrap_error(&operation_id, request, error.clone());
                    return LookupFeedback::Error(error);
                }
            };
            match session.install_bootstrap(&operation_id, request, response.clone(), snapshot) {
                Ok(record_ordinal) => LookupFeedback::Bootstrap {
                    record_ordinal,
                    response,
                },
                Err(error) => LookupFeedback::Error(error),
            }
        }
        Err(error) => {
            session.record_bootstrap_error(&operation_id, request, error.clone());
            LookupFeedback::Error(error)
        }
    }
}

fn bootstrap_error(
    error: crate::game_information_binding::game_information_bootstrap::BootstrapError,
) -> LookupError {
    match error {
        crate::game_information_binding::game_information_bootstrap::BootstrapError::Bounds => {
            LookupError::Bounds
        }
        crate::game_information_binding::game_information_bootstrap::BootstrapError::Scope => {
            LookupError::Scope
        }
        crate::game_information_binding::game_information_bootstrap::BootstrapError::Unavailable
        | crate::game_information_binding::game_information_bootstrap::BootstrapError::Ambiguous => {
            LookupError::MissingCapability
        }
        crate::game_information_binding::game_information_bootstrap::BootstrapError::Invalid => {
            LookupError::Invalid
        }
    }
}
