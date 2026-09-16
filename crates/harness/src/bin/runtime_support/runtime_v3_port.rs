// SPDX-License-Identifier: MIT


include!("runtime_v3_port_helpers.rs");
#[path = "runtime_v3_game_information.rs"]
mod game_information;

impl RuntimeV3Port {
    #[cfg(test)]
    fn new_with_telemetry(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
    ) -> Result<Self, String> {
        Self::new(config, telemetry, None)
    }

    pub(super) fn new_with_store(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
        durable: durable::DurableHandle,
    ) -> Result<Self, String> {
        Self::new_with_lookup_owner(config, telemetry, Some(durable), None)
    }

    fn new_with_store_and_lookup_owner(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
        durable: durable::DurableHandle,
        lookup_policy_owner: Option<Arc<game_information_owner::RuntimeGameInformationOwner>>,
    ) -> Result<Self, String> {
        Self::new_with_lookup_owner(config, telemetry, Some(durable), lookup_policy_owner)
    }

    pub(super) fn allocated_lease_binding(
        &self,
    ) -> Result<sts2_harness::RuntimeLeaseBinding, sts2_harness::PortError> {
        if !self.allocated || self.released {
            return Err(wire::port_error(
                "runtime_lease_binding_unavailable",
                "gateway lease is not currently allocated",
                false,
            ));
        }
        let (lease_id, lease_epoch) = self.recovery_authority.as_ref().map_or_else(
            || (self.config.lease_id.as_str(), self.config.lease_epoch),
            |authority| (authority.lease_id.as_str(), authority.lease_epoch),
        );
        if lease_id != self.config.lease_id || lease_epoch != self.config.lease_epoch {
            return Err(wire::port_error(
                "runtime_lease_binding_mismatch",
                "validated allocation and active recovery authority disagree",
                false,
            ));
        }
        Ok(sts2_harness::RuntimeLeaseBinding {
            instance_id: self.config.instance_id.clone(),
            session_id: self.config.session_id.clone(),
            run_id: self.config.run_id.clone(),
            lease_id: lease_id.to_owned(),
            lease_epoch,
        })
    }

    #[cfg(test)]
    fn new(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
        durable: Option<durable::DurableHandle>,
    ) -> Result<Self, String> {
        Self::new_with_lookup_owner(config, telemetry, durable, None)
    }

    fn new_with_lookup_owner(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
        durable: Option<durable::DurableHandle>,
        lookup_policy_owner: Option<Arc<game_information_owner::RuntimeGameInformationOwner>>,
    ) -> Result<Self, String> {
        let gateway = GatewayClient::new(&config)?;
        Ok(Self {
            config,
            gateway,
            lookup_binding_required: false,
            lookup_binding: None,
            lookup_binding_discovery_request: None,
            lookup_policy_owner,
            lookup_policy_binding: None,
            lookup_session: None,
            lookup_corpus: None,
            lookup_replay_mode: false,
            mcp: None,
            seeded_mcp: None,
            seeded_receipt: None,
            expert_mcp: None,
            allocated: false,
            released: false,
            continuation_prelaunched: false,
            continuation_adopted: false,
            continuation_boundary_verified: false,
            continuation_adopted_owner: None,
            next_rpc_id: 1,
            expert_next_rpc_id: 1,
            generation: 0,
            current_state: None,
            current_actions: None,
            catalog: None,
            catalog_raw: None,
            payloads: BTreeMap::new(),
            rest_selector_actions: None,
            rest_selector_payloads: BTreeMap::new(),
            rest_selector_value: None,
            operations: BTreeMap::new(),
            reconnect_attempts: 0,
            telemetry,
            durable,
            last_response_text: None,
            recovery_authority: None,
            recovery: None,
            recovery_context: None,
            recovery_rpc_id: 1,
            lifecycle_authority: lifecycle_authority::RuntimeLifecycleAuthorityState::default(),
            continuation_owner_claim: None,
        })
    }

    fn lifecycle_authority_state(
        &self,
    ) -> lifecycle_authority::RuntimeLifecycleAuthorityState {
        self.lifecycle_authority.clone()
    }

    fn arm_continuation_owner_claim(
        &mut self,
        context: continuation_owner::ContinuationOwnerClaimContext,
    ) -> Result<(), String> {
        if self.allocated || self.continuation_owner_claim.is_some() {
            return Err(String::from(
                "continuation owner claim must be armed once before runtime allocation",
            ));
        }
        self.continuation_owner_claim = Some(context);
        Ok(())
    }

    fn preflight_continuation_launch(&mut self) -> Result<(), String> {
        if self.continuation_owner_claim.is_none() || self.allocated {
            return Err(String::from(
                "continuation owner preflight is unavailable or already allocated",
            ));
        }
        match EpisodeRuntimePort::launch(self) {
            Ok(()) => {
                self.continuation_prelaunched = true;
                Ok(())
            }
            Err(error) => {
                let close = self.close_mcp_processes();
                let release = self.release_lease_inner();
                Err(wire::combine_cleanup(error.to_string(), close, release))
            }
        }
    }

    /// Reattaches to the selected branch's existing live owner without allocating or initializing
    /// a new host run. The first Runtime-v3 observation must still match the durable boundary.
    fn preflight_continuation_resume(&mut self) -> Result<(), String> {
        if self.continuation_owner_claim.is_none() || self.allocated {
            return Err(String::from(
                "selected-branch owner adoption is unavailable or already allocated",
            ));
        }
        let durable = self
            .durable
            .as_ref()
            .ok_or_else(|| String::from("selected-branch resume has no durable execution store"))?;
        if !durable.has_resume_boundary()? {
            return Err(String::from(
                "selected-branch resume has no durable verified observation boundary",
            ));
        }
        if let Some(operation) = durable.pending_operations()?.first() {
            return Err(format!(
                "selected-branch resume is blocked by unresolved durable operation {} ({:?}); reconcile it through the authoritative operation lookup before retrying",
                operation.intent.operation_id, operation.state
            ));
        }
        let context = self
            .continuation_owner_claim
            .as_ref()
            .ok_or_else(|| String::from("selected-branch owner claim disappeared"))?;
        let expected_owner = context
            .claim
            .owner_json
            .as_deref()
            .map(|owner| super::gateway_json::parse(owner.as_bytes()))
            .transpose()
            .map_err(|_| String::from("persisted selected-branch owner fence is invalid"))?;
        let allocation = continuation_owner::adopt_current_owner(&self.config, context)?;
        allocation.apply_current_lease(&mut self.config);
        self.recovery_authority = allocation.recovery_authority;
        let authority = self.recovery_authority.as_ref().ok_or_else(|| {
            String::from("selected-branch adopt response omitted current recovery authority")
        })?;
        self.recovery_context = Some(
            recovery::RecoveryContext::from_authority(authority, &self.config)
                .map_err(|error| format!("adopted recovery authority is invalid: {error}"))?,
        );
        self.continuation_adopted_owner = expected_owner.clone();
        self.continuation_adopted = true;
        self.continuation_boundary_verified = false;
        self.require_lifecycle_lease_authority()
            .map_err(|error| format!("adopted lease authority is invalid: {error}"))?;
        // The existing lease is retained even if MCP launch or boundary verification fails.
        self.allocated = true;
        let binding = self
            .allocated_lease_binding()
            .map_err(|error| format!("adopted runtime lease binding is invalid: {error}"))?;
        let owner = expected_owner.ok_or_else(|| {
            String::from("persisted selected-branch owner fence is unavailable")
        })?;
        if binding.instance_id != owner["instance_id"]
            || binding.session_id != owner["session_id"]
            || binding.lease_id != owner["lease_id"]
            || binding.lease_epoch != owner["lease_epoch"].as_u64().unwrap_or_default()
        {
            return Err(String::from(
                "adopted runtime lease binding differs from the persisted gateway owner fence",
            ));
        }
        if let Err(error) = self.launch_mcp() {
            let close = self.close_mcp_processes();
            return Err(wire::combine_cleanup(error, close, Ok(())));
        }
        self.continuation_prelaunched = true;
        Ok(())
    }

    fn cleanup_continuation_preflight(&mut self) -> Result<(), String> {
        self.continuation_prelaunched = false;
        let close = self.close_mcp_processes();
        let release = self.release_lease_inner();
        match (close, release) {
            (Ok(()), Ok(())) => Ok(()),
            (close, release) => Err(wire::combine_cleanup(
                String::from("continuation preflight cleanup failed"),
                close,
                release,
            )),
        }
    }

    fn mark_adopted_boundary_verified(&mut self) -> Result<(), String> {
        if !self.continuation_adopted || self.continuation_boundary_verified {
            return Ok(());
        }
        let durable = self
            .durable
            .as_ref()
            .ok_or_else(|| String::from("adopted continuation lost its durable boundary"))?;
        if durable.has_resume_boundary()? {
            return Err(String::from(
                "first observation did not consume the durable resume boundary",
            ));
        }
        self.continuation_boundary_verified = true;
        Ok(())
    }

    pub(super) fn durable_handle(&self) -> Option<durable::DurableHandle> {
        self.durable.clone()
    }

}

include!("runtime_v3_port_transport.rs");
include!("runtime_v3_lifecycle_port_hooks.rs");
include!("runtime_v3_port_durable_receipts.rs");
