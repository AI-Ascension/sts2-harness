// SPDX-License-Identifier: MIT

use super::support::authorize;
use super::*;

impl ManagementService {
    pub fn replay(
        &self,
        actor: &AuthContext,
        request: ReplayRequest,
    ) -> Result<ReplayResponse, ManagementError> {
        schema_is(&request.schema_version, MANAGEMENT_SCHEMA_VERSION)?;
        validate_identifier("run_id", &request.run_id)?;
        authorize(actor, "workflow:read", Some(&request.run_id))?;
        if !request.offline {
            return Err(ManagementError::invalid(
                "offline_required",
                "replay requires offline=true",
            ));
        }
        let snapshot = self.store.get_run(&request.run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let page = self.store.events(
            &request.run_id,
            0,
            super::super::contract::MAX_EVENTS_PER_PAGE,
        )?;
        let mut events = page.events;
        let mut cursor = page.next_after_sequence;
        while let Some(newest) = page.newest_sequence {
            if cursor >= newest {
                break;
            }
            let next = self.store.events(
                &request.run_id,
                cursor,
                super::super::contract::MAX_EVENTS_PER_PAGE,
            )?;
            if next.events.is_empty() {
                break;
            }
            cursor = next.next_after_sequence;
            events.extend(next.events);
        }
        let result = self.replay.replay(&request, &snapshot, &events)?;
        if !result.matched {
            return Err(ManagementError::replay(
                "replay_divergence",
                result
                    .first_divergence
                    .as_ref()
                    .map(|divergence| format!("replay diverged at {}", divergence.path))
                    .unwrap_or_else(|| "replay diverged".to_owned()),
            ));
        }
        Ok(ReplayResponse {
            schema_version: REPLAY_SCHEMA_VERSION.to_owned(),
            workflow_run_id: request.run_id,
            matched: true,
            compared_events: result.compared_events,
            first_divergence: None,
        })
    }

    pub fn export(
        &self,
        actor: &AuthContext,
        request: ExportRequest,
    ) -> Result<ExportResponse, ManagementError> {
        schema_is(&request.schema_version, MANAGEMENT_SCHEMA_VERSION)?;
        validate_identifier("run_id", &request.run_id)?;
        authorize(actor, "workflow:export", Some(&request.run_id))?;
        if !request.redacted {
            return Err(ManagementError::forbidden(
                "redaction_required",
                "management export must be redacted",
            ));
        }
        Ok(self.store.export(&request.run_id, true)?)
    }

    pub fn capabilities(&self, actor: &AuthContext) -> Result<CapabilityResponse, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        Ok(CapabilityResponse {
            schema_version: super::super::contract::CAPABILITIES_SCHEMA_VERSION.to_owned(),
            capabilities: self.capabilities.capabilities()?,
        })
    }

    pub fn health(&self) -> HealthResponse {
        HealthResponse {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            status: "ok".to_owned(),
        }
    }
}
