// SPDX-License-Identifier: MIT

use crate::worker_handoff::json::decode;
use serde_json::Value;

fn parse_bootstrap_payload(payload: &[u8]) -> Result<Bootstrap, String> {
    let value = decode(payload).map_err(|_| String::from("worker bootstrap JSON is invalid"))?;
    let object = value
        .as_object()
        .ok_or_else(|| String::from("worker bootstrap schema is invalid"))?;
    exact_fields(
        object,
        &[
            "version",
            "launch_nonce",
            "watchdog_boot_id",
            "component_id",
            "expected_peer",
        ],
    )?;
    if object.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(String::from("worker bootstrap version is unsupported"));
    }
    let launch_nonce = uuid4_string(object, "launch_nonce")?;
    let watchdog_boot_id = uuid4_string(object, "watchdog_boot_id")?;
    let component_id = peer_string(object, "component_id")?.to_owned();
    validate_component(&component_id)?;
    let peer_object = object
        .get("expected_peer")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("worker bootstrap peer is invalid"))?;
    exact_fields(
        peer_object,
        &[
            "platform",
            "pid",
            "creation_token",
            "executable",
            "executable_sha256",
            "session_id",
            "sid",
        ],
    )?;
    if peer_string(peer_object, "platform")? != "windows" {
        return Err(String::from("worker bootstrap peer platform is invalid"));
    }
    let peer = WindowsPeer {
        pid: bounded_u32(peer_object, "pid", true)?,
        creation_token: parse_creation_token(peer_string(peer_object, "creation_token")?)?,
        executable: PathBuf::from(peer_string(peer_object, "executable")?),
        executable_sha256: peer_string(peer_object, "executable_sha256")?.to_owned(),
        session_id: bounded_u32(peer_object, "session_id", false)?,
        sid: peer_string(peer_object, "sid")?.to_owned(),
    };
    validate_windows_peer(&peer)?;
    Ok(Bootstrap {
        launch_nonce,
        watchdog_boot_id,
        component_id,
        peer,
    })
}

fn exact_fields(object: &serde_json::Map<String, Value>, fields: &[&str]) -> Result<(), String> {
    if object.len() != fields.len()
        || object
            .keys()
            .any(|key| !fields.iter().any(|field| *field == key))
    {
        return Err(String::from("worker bootstrap schema is not closed"));
    }
    Ok(())
}

fn peer_string<'a>(object: &'a serde_json::Map<String, Value>, name: &str) -> Result<&'a str, String> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("worker bootstrap field is invalid"))
}

fn bounded_u32(
    object: &serde_json::Map<String, Value>,
    name: &str,
    positive: bool,
) -> Result<u32, String> {
    let value = object
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| String::from("worker bootstrap number is invalid"))?;
    if positive && value == 0 {
        return Err(String::from("worker bootstrap number is invalid"));
    }
    Ok(value)
}

fn uuid4_string(object: &serde_json::Map<String, Value>, name: &str) -> Result<String, String> {
    let value = peer_string(object, name)?;
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|_| String::from("worker bootstrap UUID is invalid"))?;
    if parsed.get_version_num() != 4
        || parsed.get_variant() != uuid::Variant::RFC4122
        || parsed.to_string() != value
    {
        return Err(String::from("worker bootstrap UUID is invalid"));
    }
    Ok(value.to_owned())
}

fn validate_windows_peer(peer: &WindowsPeer) -> Result<(), String> {
    if peer.pid == 0 || peer.creation_token == 0 {
        return Err(String::from("worker bootstrap peer number is invalid"));
    }
    validate_windows_path(&peer.executable, "worker bootstrap executable")?;
    validate_digest(&peer.executable_sha256)?;
    validate_sid(&peer.sid)
}
