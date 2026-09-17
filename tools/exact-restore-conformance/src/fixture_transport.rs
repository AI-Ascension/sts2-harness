// SPDX-License-Identifier: MIT
use super::*;
pub(super) fn bootstrap(
    address: &str,
    token: &str,
    caller: &str,
    deployment: &str,
    instance: &str,
    incarnation: &str,
) -> Result<Value, String> {
    let frame = recovery_frame(
        "bootstrap_request",
        "bootstrap",
        caller,
        json!({
            "deployment_id": deployment, "instance_id": instance,
            "instance_incarnation": incarnation,
            "release": {"release_digest": ZERO_DIGEST, "config_digest": ZERO_DIGEST,
                        "profile_digest": ZERO_DIGEST, "runtime_v3_schema_digest": RUNTIME_V3_SCHEMA},
            "lease_policy": {"ttl_seconds": 30, "renewal_interval_seconds": 10}
        }),
    );
    let (status, response) = http::post(
        address,
        "/v1/recovery/bootstrap",
        token,
        Some("bootstrap"),
        &frame,
    )?;
    if status != 200 {
        return Err(format!("bootstrap failed: {status} {response}"));
    }
    let boot = response["payload"]["boot"].clone();
    if boot.is_null() {
        Err(String::from("bootstrap omitted boot"))
    } else {
        Ok(boot)
    }
}

pub(super) fn host_fence(
    address: &str,
    token: &str,
    caller: &str,
    boot: &Value,
) -> Result<(), String> {
    let frame = recovery_frame(
        "host_fence_request",
        "host_fence",
        caller,
        json!({"boot": boot}),
    );
    let (status, response) = http::post(
        address,
        "/v1/recovery/host-fence",
        token,
        Some("host_fence"),
        &frame,
    )?;
    if status == 200 {
        Ok(())
    } else {
        Err(format!("host-fence failed: {status} {response}"))
    }
}

pub(super) fn recovery_frame(
    kind: &str,
    capability: &str,
    principal: &str,
    payload: Value,
) -> Value {
    json!({
        "contract": "watchdog-recovery-v1", "schema_digest": RECOVERY_SCHEMA,
        "message_id": Uuid::new_v4().to_string(), "correlation_id": Uuid::new_v4().to_string(),
        "sent_at": "2026-09-17T00:00:00Z",
        "actor": {"principal_id": principal, "role": "harness"},
        "auth": {"principal_id": principal, "capability": capability, "proof": null},
        "kind": kind, "payload": payload
    })
}

pub(super) fn required_string(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("recovery authority omitted string field {key}"))
}

pub(super) fn required_u64(value: &Value, key: &str) -> Result<u64, String> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("recovery authority omitted integer field {key}"))
}

pub(super) fn run_harness(
    paths: &Paths,
    env: &[(String, String)],
    log: &Path,
) -> Result<i32, String> {
    let output = std::fs::File::create(log).map_err(|error| error.to_string())?;
    let error = output.try_clone().map_err(|error| error.to_string())?;
    let mut command = Command::new("cargo");
    command
        .current_dir(&paths.harness_root)
        .env_remove("STS2_EXACT_NATIVE_UNSUPPORTED")
        .env_remove("STS2_EXACT_COMMIT_UNKNOWN")
        .env_remove("STS2_EXACT_LOOKUP_UNKNOWN_ONCE")
        .envs(env.iter().map(|(name, value)| (name, value)))
        .arg("test").arg("--locked").arg("--package").arg("sts2-harness")
        .arg("--bin").arg("sts2-harness-runtime")
        .arg("runtime_support::exact_restore::tests::actual_production_entrypoint_exact_restore_matrix_case")
        .arg("--").arg("--ignored").arg("--exact")
        .stdout(Stdio::from(output)).stderr(Stdio::from(error));
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn harness test: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Ok(status.code().unwrap_or(1));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(String::from(
                "harness exact-restore test exceeded 120 seconds",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| error.to_string())
}

pub(super) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as usize;
        let second = chunk.get(1).copied().unwrap_or(0) as usize;
        let third = chunk.get(2).copied().unwrap_or(0) as usize;
        output.push(ALPHABET[first >> 2] as char);
        output.push(ALPHABET[((first & 3) << 4) | (second >> 4)] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((second & 15) << 2) | (third >> 6)] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[third & 63] as char
        } else {
            '='
        });
    }
    output
}

pub(super) fn hex(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(ALPHABET[(byte >> 4) as usize] as char);
        output.push(ALPHABET[(byte & 0x0f) as usize] as char);
    }
    output
}
