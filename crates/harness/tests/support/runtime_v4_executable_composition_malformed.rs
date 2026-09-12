// SPDX-License-Identifier: MIT

pub(crate) fn assert_malformed_envelope_rejected(
    result: &ScenarioResult,
) -> Result<(), Box<dyn std::error::Error>> {
    if result.runtime.status.code() != Some(2) {
        return Err(format!(
            "malformed envelope exit: {:?}",
            result.runtime.status.code()
        )
        .into());
    }
    let methods: Vec<&str> = result
        .ledger
        .requests
        .iter()
        .map(|request| request.method.as_str())
        .collect();
    if !result.ledger.errors.is_empty()
        || paths(&result.ledger) != ["/api/v3/runtime/state", "/api/v4/runtime/expert-state"]
        || methods != ["GET", "GET"]
        || result.ledger.responses.len() != 2
        || result.ledger.responses[1].status != 200
    {
        return Err(format!(
            "malformed envelope was not rejected before action forwarding: errors={:?}, paths={:?}, methods={methods:?}, responses={:?}, stderr={}",
            result.ledger.errors,
            paths(&result.ledger),
            result.ledger.responses,
            String::from_utf8_lossy(&result.runtime.stderr),
        )
        .into());
    }
    Ok(())
}
