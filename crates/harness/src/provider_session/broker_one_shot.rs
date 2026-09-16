// SPDX-License-Identifier: MIT

use super::super::types::*;
use super::ProviderSessionBroker;

impl ProviderSessionBroker {
    /// Admits a fresh binding for the receipt-carrying Exo one-shot adapter.
    ///
    /// The pending native-thread reference is an internal marker, not a claimed native identity.
    /// Threaded profiles cannot call this path.
    pub fn admit_one_shot_binding(
        &mut self,
        owner_token: &str,
        binding_id: &str,
        expires_at: &str,
    ) -> Result<SessionBinding, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self.policy.allows_execution()
            || !self
                .capabilities
                .enabled_methods
                .iter()
                .any(|method| method == "turn/start")
            || self
                .capabilities
                .enabled_methods
                .iter()
                .any(|method| method == "thread/start")
            || !valid_id(binding_id)
            || self.bindings.contains_key(binding_id)
            || self.bindings.values().any(SessionBinding::executable)
        {
            return Err(SessionError::Unsupported);
        }
        self.ensure_not_expired(expires_at)?;
        let mut binding = SessionBinding::candidate(
            binding_id,
            self.scope.clone(),
            "one-shot",
            SessionPurpose::Executable,
            self.policy.credential_realm_ref.clone(),
            self.policy.profile_sha256.clone(),
            expires_at,
        )?;
        binding.state = BindingState::Active;
        binding.game_dispatch_capability = true;
        binding.owner_epoch = self.owner_epoch;
        binding.validate()?;
        self.bindings.insert(binding.binding_id.clone(), binding.clone());
        Ok(binding)
    }
}
