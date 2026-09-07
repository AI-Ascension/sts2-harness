// SPDX-License-Identifier: MIT

impl RuntimeV3Port {

    /// Rebind a settled Runtime-v3 result to the expert observation before it reaches the
    /// provider. The normal endpoint remains the mutation authority for ordinary actions, while
    /// the expert endpoint supplies the richer postcondition view.
    pub(super) fn compose_receipt_after(
        &mut self,
        receipt: TransitionReceipt,
    ) -> Result<TransitionReceipt, String> {
        let Some(after) = receipt.after().cloned() else {
            return Ok(receipt);
        };
        let after = self.compose_current_observation(after)?;
        Ok(TransitionReceipt::new(
            receipt.operation_id().to_owned(),
            receipt.action().clone(),
            receipt.status(),
            Some(after),
            receipt.effect_kind().map(str::to_owned),
            receipt.error_code().map(str::to_owned),
        ))
    }

    /// Replace the ordinary wait observation with the matching expert observation while keeping
    /// the barrier outcome and effect witness intact.
    pub(super) fn compose_wait_sample(&mut self, sample: WaitSample) -> Result<WaitSample, String> {
        let Some(after) = sample.observation().cloned() else {
            return Ok(sample);
        };
        let after = self.compose_current_observation(after)?;
        let outcome = sample.outcome();
        let effect_kind = sample.effect_kind().map(str::to_owned);
        let mut composed = WaitSample::new(outcome, Some(after));
        if let Some(effect_kind) = effect_kind {
            composed = composed.with_effect_kind(effect_kind);
        }
        Ok(composed)
    }

}
