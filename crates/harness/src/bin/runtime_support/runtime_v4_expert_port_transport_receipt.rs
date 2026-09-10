// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    fn expert_result_receipt(
        &mut self,
        result: RuntimeV4ExpertActionResult,
        _request: &RuntimeV4ExpertActionRequest,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, String> {
        let status = match result.status() {
            RuntimeV4ExpertActionStatus::Accepted => DispatchStatus::Accepted,
            RuntimeV4ExpertActionStatus::Settled => DispatchStatus::Settled,
            RuntimeV4ExpertActionStatus::Rejected => DispatchStatus::Rejected,
            RuntimeV4ExpertActionStatus::Unknown => DispatchStatus::Unknown,
            RuntimeV4ExpertActionStatus::Cancelled => DispatchStatus::Cancelled,
        };
        let after = if status == DispatchStatus::Settled {
            let expert =
                RuntimeV4ExpertObservation::from_value(result.as_value()["observation"].clone())
                    .map_err(|error| {
                        format!("expert settlement observation is invalid: {error}")
                    })?;
            let composed = expert_only_observation(&expert)?;
            self.install_composed(&composed)?;
            Some(composed.observation)
        } else {
            None
        };
        let effect_kind =
            (status == DispatchStatus::Settled).then(|| String::from("potion_use_settled"));
        let error_code = result.as_value()["error_code"].as_str().map(str::to_owned);
        Ok(TransitionReceipt::new(
            identity.operation_id.clone(),
            action.clone(),
            status,
            after,
            effect_kind,
            error_code,
        ))
    }
}
