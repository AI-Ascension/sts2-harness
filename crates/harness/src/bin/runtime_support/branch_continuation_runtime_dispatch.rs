// SPDX-License-Identifier: MIT

/// Typed owner boundary for the two durable branch continuation strategies.
pub(crate) trait BranchContinuationEffectPort {
    /// Effect result after destination restore evidence has been verified.
    type Output;

    /// Restores the exact checkpoint into a fresh destination and verifies destination evidence.
    fn exact_restore(
        &mut self,
        selected: &mut SelectedBranchContinuation,
    ) -> Result<Self::Output, String>;

    /// Replays the retained prefix and continues the same destination after its boundary verifies.
    fn prefix_replay(
        &mut self,
        selected: &mut SelectedBranchContinuation,
        prefix: &[u8],
    ) -> Result<Self::Output, String>;
}

/// Dispatches the persisted strategy through the typed owner port.
pub(crate) fn dispatch<P: BranchContinuationEffectPort>(
    selected: &mut SelectedBranchContinuation,
    port: &mut P,
) -> Result<P::Output, String> {
    match selected.strategy() {
        BranchContinuationStrategyPlan::ExactRestore { .. } => port.exact_restore(selected),
        BranchContinuationStrategyPlan::PrefixReplay { .. } => {
            let prefix = selected
                .replay_prefix()
                .ok_or_else(|| String::from("verified replay prefix bytes are unavailable"))?
                .to_vec();
            port.prefix_replay(selected, &prefix)
        }
    }
}
