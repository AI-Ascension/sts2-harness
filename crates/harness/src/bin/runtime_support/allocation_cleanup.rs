// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;

use super::{RuntimeConfig, identity_headers, validate_allocation};

pub(in super::super) fn validate_or_release_allocation(
    allocation: Result<Value, String>,
    config: &RuntimeConfig,
    release: impl FnOnce(BTreeMap<String, String>) -> Result<Value, String>,
) -> Result<(), String> {
    let validation = match &allocation {
        Ok(value) => validate_allocation(value, config),
        Err(error) => Err(error.clone()),
    };
    let Err(error) = validation else {
        return Ok(());
    };
    let mut headers = identity_headers(config, "release-0001");
    if let Ok(value) = allocation
        && let Some((lease, epoch)) = attributable_lease(&value, config)
    {
        headers.insert(String::from("x-sts2-lease-id"), lease.to_owned());
        headers.insert(String::from("x-sts2-lease-epoch"), epoch.to_string());
    }
    match release(headers) {
        Ok(value) if value["status"] == "released" => Err(error),
        Ok(_) => Err(format!(
            "{error}; allocation cleanup did not confirm release"
        )),
        Err(cleanup) => Err(format!("{error}; allocation cleanup failed: {cleanup}")),
    }
}

fn attributable_lease<'a>(value: &'a Value, config: &RuntimeConfig) -> Option<(&'a str, u64)> {
    for (key, expected) in [
        ("instance_id", config.instance_id.as_str()),
        ("caller_id", config.caller_id.as_str()),
        ("session_id", config.session_id.as_str()),
    ] {
        if value[key].as_str() != Some(expected) {
            return None;
        }
    }
    let lease = value["lease_id"].as_str()?;
    let epoch = value["lease_epoch"].as_u64()?;
    if lease.is_empty()
        || lease.len() > 128
        || lease.contains("..")
        || !lease.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
        || epoch == 0
        || epoch > 9_007_199_254_740_991
    {
        return None;
    }
    Some((lease, epoch))
}
