// SPDX-License-Identifier: MIT

/// Binds the host's recovery reason to a composed expert observation.
///
/// A recovery state crosses the expert composition the same way a playable one does: the
/// observation is rebuilt from the projection's identity and stage, so the reason token that the
/// runtime-v3 parse path reads out of `state.code` has to be re-bound here. Without it the
/// `runtime-v4-expert` and `runtime-v4-expert-rest-action` profiles report the anonymous recovery
/// sentence for a condition the runtime-v3 profile names.
fn bind_recovery_code(observation: EpisodeObservation) -> Result<EpisodeObservation, String> {
    if observation.stage() != EpisodeStage::Recovery {
        return Ok(observation);
    }
    let code = observation
        .fair_play()
        .as_value()
        .get("state")
        .and_then(Value::as_object)
        .and_then(|state| state.get("code"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| String::from("expert recovery projection omitted its reason code"))?;
    observation
        .with_recovery_code(code)
        .map_err(|error| format!("expert recovery observation failed validation: {error}"))
}
