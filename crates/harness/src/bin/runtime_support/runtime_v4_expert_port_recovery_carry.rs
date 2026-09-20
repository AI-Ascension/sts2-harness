// SPDX-License-Identifier: MIT

/// Carries the runtime-v3 recovery reason across an expert composition.
///
/// The expert projection names its own `recovery` state but carries no `code` field for it, so a
/// composed observation cannot re-derive the reason the runtime-v3 baseline already parsed and
/// validated. Rebuilding the observation without that token reports the generic "requires
/// recovery" sentence on every expert profile, which is the outcome the token exists to remove.
/// The parsed baseline stays the only owner of the token, so carry it instead of re-deriving it
/// from host-adjacent text that never passed the identity rule.
fn carry_recovery_code(
    observation: EpisodeObservation,
    code: Option<&str>,
) -> Result<EpisodeObservation, String> {
    let Some(code) = code else {
        return Ok(observation);
    };
    observation.with_recovery_code(code).map_err(|error| {
        format!("composed expert observation refused the baseline recovery code: {error}")
    })
}
