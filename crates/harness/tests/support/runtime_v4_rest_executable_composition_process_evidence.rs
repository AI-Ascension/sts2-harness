// SPDX-License-Identifier: MIT

pub(crate) fn write_evidence(
    result: &ScenarioResult,
    operation: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::var_os("STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR") else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    fs::create_dir_all(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    // Evidence retention is deliberately limited to this synthetic fixture and the explicitly
    // supplied synthetic binaries. Every capture is bounded and each file is private; callers
    // must still keep the directory private because env_clear does not sanitize child output.
    write_private(&root, "runtime.stdout", &result.runtime.stdout)?;
    write_private(&root, "runtime.stderr", &result.runtime.stderr)?;
    write_private(&root, "gateway.stdout", &result.gateway.stdout)?;
    write_private(&root, "gateway.stderr", &result.gateway.stderr)?;
    let summary = Value::Array(
        result
            .ledger
            .requests
            .iter()
            .zip(&result.ledger.responses)
            .map(|(request, response)| {
                json!({
                    "method": request.method,
                    "path": request.path,
                    "operation_id": request.body["operation_id"],
                    "action_id": request.body["action"]["action_id"],
                    "state_id": request.body["state_id"],
                    "generation": request.body["generation"],
                    "response_status": response.status,
                    "response_operation_id": response.body["operation_id"],
                    "response_state_id": response.body["state_id"],
                    "response_generation": response.body["generation"],
                    "response_status_value": response.body["status"],
                    "response_effect_witness": response.body["effect_witness"]
                })
            })
            .collect(),
    );
    write_private(
        &root,
        "downstream.json",
        &serde_json::to_vec_pretty(&summary)?,
    )?;
    write_private(
        &root,
        "result.json",
        &serde_json::to_vec_pretty(&json!({
            "status": "confirmed",
            "scope": "source-derived executable REST composition",
            "operation_id": operation,
            "runtime_exit": result.runtime.status.code(),
            "gateway_exit": result.gateway.status.code(),
            "provider": "synthetic bounded bridge",
            "game": "synthetic REST downstream"
        }))?,
    )?;
    Ok(())
}

fn write_private(root: &Path, name: &str, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err(format!("evidence file {name} exceeds the bounded output limit").into());
    }
    let path = root.join(name);
    fs::write(&path, bytes)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}
