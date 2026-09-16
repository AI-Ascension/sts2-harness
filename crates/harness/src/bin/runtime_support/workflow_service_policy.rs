// SPDX-License-Identifier: MIT

use super::*;

const PROVIDER_POLICY_CONFIGURATION_SCHEMA: &str = "ascension.workflow-provider-policy-config.v1";
const MAX_PROVIDER_POLICY_CONFIGURATION_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProviderPolicyConfiguration {
    schema_version: String,
    store_path: std::path::PathBuf,
    key_reference: String,
    pub(super) scope: SessionScope,
    pub(super) capabilities: NativeCapabilities,
    selected_profile: String,
}

impl ProviderPolicyConfiguration {
    pub(super) fn from_environment() -> Result<Self, String> {
        let bytes = required("STS2_WORKFLOW_PROVIDER_POLICY_CONFIG")?;
        if bytes.len() > MAX_PROVIDER_POLICY_CONFIGURATION_BYTES {
            return Err(String::from(
                "STS2_WORKFLOW_PROVIDER_POLICY_CONFIG exceeds its byte bound",
            ));
        }
        let configuration: Self = serde_json::from_str(&bytes).map_err(|_| {
            String::from(
                "STS2_WORKFLOW_PROVIDER_POLICY_CONFIG must be a closed provider-policy configuration",
            )
        })?;
        if configuration.schema_version != PROVIDER_POLICY_CONFIGURATION_SCHEMA
            || !configuration.scope.valid()
            || !valid_environment_name(&configuration.key_reference)
            || configuration.selected_profile != configuration.capabilities.profile_id
        {
            return Err(String::from(
                "STS2_WORKFLOW_PROVIDER_POLICY_CONFIG contains an invalid provider-policy binding",
            ));
        }
        configuration.capabilities.validate().map_err(|_| {
            String::from(
                "STS2_WORKFLOW_PROVIDER_POLICY_CONFIG contains invalid native capabilities",
            )
        })?;
        Ok(configuration)
    }

    pub(super) fn open_owner(&self) -> Result<ProviderSessionPolicyOwner, String> {
        let mut key = provider_policy_key(&self.key_reference)?;
        let store =
            ProviderSessionMetadataStore::encrypted(&self.store_path, key, self.scope.clone());
        key.zeroize();
        let store = store
            .map_err(|_| String::from("provider-policy metadata store configuration is invalid"))?;
        ProviderSessionPolicyOwner::open(store, self.scope.clone(), self.capabilities.clone())
            .map_err(|_| String::from("provider-policy owner could not be opened"))
    }
}
