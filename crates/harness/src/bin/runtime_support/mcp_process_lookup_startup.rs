// SPDX-License-Identifier: MIT

use sts2_harness::game_information_binding::{LookupBindingOperation, LookupBindingRequest};

const MAX_LOOKUP_BINDING_DISCOVERY_REQUEST_BYTES: usize = 2048;
const LOOKUP_BINDING_DISCOVERY_REQUEST_ENV: &str = "STS2_LOOKUP_BINDING_DISCOVERY_REQUEST_JSON";

impl McpProcess {
    pub(super) fn spawn_profile_with_lookup_discovery_request(
        config: &RuntimeConfig,
        profile: &str,
        request: &LookupBindingRequest,
    ) -> Result<Self, String> {
        if request.operation != LookupBindingOperation::Discovery
            || request.authority_epoch == 0
            || request.authority_epoch > 9_007_199_254_740_991
            || request.correlation_id != "game-information-binding-discovery"
        {
            return Err(String::from(
                "lookup-binding startup request is not a valid discovery identity",
            ));
        }
        let bytes = serde_json::to_vec(request)
            .map_err(|_| String::from("lookup-binding startup request could not be encoded"))?;
        if bytes.len() > MAX_LOOKUP_BINDING_DISCOVERY_REQUEST_BYTES {
            return Err(String::from(
                "lookup-binding startup request exceeds its size bound",
            ));
        }
        let request_json = String::from_utf8(bytes)
            .map_err(|_| String::from("lookup-binding startup request is not UTF-8"))?;
        let max_response_bytes = if profile == "runtime-map-v1" {
            MAP_MAX_RESPONSE_BYTES
        } else {
            MAX_RESPONSE_BYTES
        };
        let mut command = Self::configured_command_for_profile(config, profile);
        command.env(LOOKUP_BINDING_DISCOVERY_REQUEST_ENV, request_json);
        Self::spawn_command_with_response_limit(command, EXCHANGE_TIMEOUT, max_response_bytes)
    }
}
