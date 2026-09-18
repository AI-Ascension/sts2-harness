// SPDX-License-Identifier: MIT

//! Serves the producer-generated `ascension.harness.effective-limits.v1`
//! records over the management surface for one workflow run.
//!
//! The provider-session record is built by the library producer
//! (`NativeCapabilities::effective_limit_record`) from the capability descriptor
//! the served composition admits provider sessions against; it is never a
//! hand-written copy. The route is fenced to the run's **current**
//! context-owner association: that association's boundary must name the adapter
//! and model revision the served descriptor binds, so a caller cannot read a
//! record for a run this process does not serve. The context-memory record has
//! no corpus in the served composition and stays explicitly unavailable.

use super::support::authorize;
use super::*;
use crate::effective_limits::EffectiveLimitRecord;
use crate::provider_session::NativeCapabilities;

impl ManagementService {
    /// Attaches the provider-session capability descriptor whose effective-limit
    /// record the management surface serves. The descriptor is validated first,
    /// so an invalid descriptor is refused at composition instead of published.
    pub fn with_provider_session_capabilities(
        mut self,
        capabilities: NativeCapabilities,
    ) -> Result<Self, ManagementError> {
        capabilities.validate().map_err(|error| {
            ManagementError::capability(
                "provider_session_capabilities_invalid",
                format!("provider session capability descriptor is invalid: {error}"),
            )
        })?;
        self.provider_session_capabilities = Some(capabilities);
        Ok(self)
    }

    /// The producer's `ascension.harness.effective-limits.v1` record for the
    /// provider-session surface this composition serves, for one workflow run.
    ///
    /// Metadata only: the record classifies advertised ceilings and names the
    /// descriptor digest; it carries no session content, credential or policy
    /// bytes. Requires scoped `workflow:read`, an admitted run and an owner
    /// association whose boundary is bound to the served descriptor's adapter
    /// and model revision; a composition without a descriptor stays explicitly
    /// unavailable rather than answering with a fixture.
    pub fn provider_session_effective_limits(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<EffectiveLimitRecord, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        let capabilities = self.provider_session_capabilities.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "provider_session_capabilities_unavailable",
                "the served composition holds no provider-session capability descriptor",
            )
        })?;
        let binding = self
            .current_context_owner_association(actor, run_id)?
            .binding;
        if binding.boundary.adapter_revision != capabilities.binding.adapter_revision
            || binding.boundary.model_revision != capabilities.binding.model_revision
        {
            return Err(ManagementError::conflict(
                "provider_session_capabilities_mismatch",
                "the run's current association is not bound to the served provider-session capabilities",
            ));
        }
        let record = capabilities.effective_limit_record();
        record.validate().map_err(|error| {
            ManagementError::capability(
                "effective_limit_record_invalid",
                format!("the produced effective-limit record is invalid: {error:?}"),
            )
        })?;
        Ok(record)
    }

    /// The context-memory effective-limits record for one workflow run.
    ///
    /// The served workflow composition holds no selected-memory corpus, so no
    /// `MemoryCapabilities` producer exists in-process and the record is not
    /// served. The refusal is typed (`context_memory_record_unavailable`) after
    /// the same scope and run checks as the provider-session record, so a caller
    /// can tell "not served here" apart from an unknown route or run.
    pub fn context_memory_effective_limits(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<EffectiveLimitRecord, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        Err(ManagementError::unavailable(
            "context_memory_record_unavailable",
            "the served composition holds no context-memory corpus; its effective-limit record is not served",
        ))
    }
}
