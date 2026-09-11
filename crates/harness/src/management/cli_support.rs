// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::{Component, Path};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::super::{
    ClientResponse, ErrorClass, ErrorResponse, MAX_JSON_BYTES, ManagementClient, decode_strict,
    validate_identifier,
};
use super::DEFAULT_LISTEN;

pub(super) struct CliOutput(pub(super) Option<Vec<u8>>);

pub(super) struct CliFailure {
    pub(super) exit_code: i32,
    pub(super) message: String,
    pub(super) json: Option<Vec<u8>>,
}

pub(super) type ParsedOptions = (Vec<String>, BTreeMap<String, String>, BTreeSet<String>);

impl CliFailure {
    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self {
            exit_code: 2,
            message: message.into(),
            json: None,
        }
    }

    pub(super) fn local(error: impl std::fmt::Display) -> Self {
        Self::invalid(error.to_string())
    }
}

pub(super) fn request_and_render(
    options: &BTreeMap<String, String>,
    method: &str,
    path: &str,
    body: Option<Vec<u8>>,
    client: ManagementClient,
) -> Result<CliOutput, CliFailure> {
    let response = client
        .request_json(method, path, body.as_deref())
        .map_err(CliFailure::local)?;
    if response.status / 100 != 2 {
        return Err(response_failure(response));
    }
    let value: Value = decode_strict(&response.body).map_err(CliFailure::local)?;
    if options.get("format").map(String::as_str) == Some("json") {
        let bytes = serde_json::to_vec(&value).map_err(CliFailure::local)?;
        Ok(CliOutput(Some(bytes)))
    } else {
        Ok(CliOutput(Some(readable_summary(&value).into_bytes())))
    }
}

pub(super) fn response_failure(response: ClientResponse) -> CliFailure {
    let exit_code = decode_strict::<ErrorResponse>(&response.body)
        .map(|error| exit_code_for_class(&error.error.class))
        .unwrap_or_else(|_| exit_code_for_http_status(response.status));
    CliFailure {
        exit_code,
        message: "management request failed".to_owned(),
        json: Some(response.body),
    }
}

pub(super) fn client_for(
    options: &BTreeMap<String, String>,
) -> Result<ManagementClient, CliFailure> {
    let default_listen =
        env::var("STS2_WORKFLOW_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.to_owned());
    let address = parse_address(options.get("listen"), &default_listen)?;
    let profile = options
        .get("auth-profile")
        .cloned()
        .or_else(|| env::var("STS2_WORKFLOW_AUTH_PROFILE").ok())
        .unwrap_or_else(|| "default".to_owned());
    validate_identifier("auth_profile", &profile).map_err(CliFailure::local)?;
    let env_name = format!(
        "STS2_WORKFLOW_TOKEN_{}",
        profile
            .chars()
            .map(|character| {
                if character == '-' {
                    '_'
                } else {
                    character.to_ascii_uppercase()
                }
            })
            .collect::<String>()
    );
    let token = env::var(&env_name).map_err(|_| {
        CliFailure::invalid(format!(
            "credential environment variable {env_name} is not set"
        ))
    })?;
    ManagementClient::new(address, token).map_err(CliFailure::local)
}

pub(super) fn read_json_file(path: &str) -> Result<Value, CliFailure> {
    let bytes = read_bounded_file(path)?;
    decode_strict(&bytes).map_err(CliFailure::local)
}

pub(super) fn read_bounded_file(path: &str) -> Result<Vec<u8>, CliFailure> {
    let metadata = fs::metadata(path).map_err(CliFailure::local)?;
    if !metadata.is_file() || metadata.len() > MAX_JSON_BYTES as u64 {
        return Err(CliFailure::invalid(
            "input file is not a bounded regular JSON file",
        ));
    }
    fs::read(path).map_err(CliFailure::local)
}

pub(super) fn validate_output_path(path: &Path) -> Result<(), CliFailure> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(CliFailure::invalid("export output path is not approved"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| CliFailure::invalid("export output parent is missing"))?;
    if !parent.exists() || !parent.is_dir() {
        return Err(CliFailure::invalid(
            "export output parent must already exist",
        ));
    }
    if path.exists() {
        return Err(CliFailure::invalid("export output already exists"));
    }
    Ok(())
}

pub(super) fn parse_options(
    args: &[String],
    _positional_flags: &[&str],
    value_flags: &[&str],
    boolean_flags: &[&str],
) -> Result<ParsedOptions, CliFailure> {
    let allowed_values = value_flags.iter().copied().collect::<BTreeSet<_>>();
    let allowed_booleans = boolean_flags.iter().copied().collect::<BTreeSet<_>>();
    let mut positionals = Vec::new();
    let mut options = BTreeMap::new();
    let mut flags = BTreeSet::new();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if let Some(name) = argument.strip_prefix("--") {
            if allowed_booleans.contains(name) {
                if !flags.insert(name.to_owned()) {
                    return Err(CliFailure::invalid("duplicate boolean option"));
                }
                index += 1;
                continue;
            }
            if !allowed_values.contains(name) {
                return Err(CliFailure::invalid(format!("unknown option --{name}")));
            }
            let value = args
                .get(index + 1)
                .ok_or_else(|| CliFailure::invalid("option value is missing"))?;
            if value.starts_with("--")
                || value.is_empty()
                || options.insert(name.to_owned(), value.clone()).is_some()
            {
                return Err(CliFailure::invalid("option value is invalid or duplicated"));
            }
            index += 2;
        } else {
            positionals.push(argument.clone());
            index += 1;
        }
    }
    if options.get("format").is_some_and(|format| format != "json") {
        return Err(CliFailure::invalid("only --format json is supported"));
    }
    Ok((positionals, options, flags))
}

pub(super) fn required_option<'a>(
    options: &'a BTreeMap<String, String>,
    name: &str,
) -> Result<&'a str, CliFailure> {
    options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| CliFailure::invalid(format!("--{name} is required")))
}

pub(super) fn parse_address(
    option: Option<&String>,
    default: &str,
) -> Result<SocketAddr, CliFailure> {
    let text = option.map(String::as_str).unwrap_or(default);
    let address = text.parse::<SocketAddr>().map_err(|_| {
        CliFailure::invalid("listen address must be a numeric IPv4/IPv6 socket address")
    })?;
    if !address.ip().is_loopback() {
        return Err(CliFailure::invalid("listen address must be loopback"));
    }
    Ok(address)
}

pub(super) fn parse_u64_option(
    options: &BTreeMap<String, String>,
    name: &str,
    default: u64,
) -> Result<u64, CliFailure> {
    options
        .get(name)
        .map(|value| value.parse::<u64>().map_err(CliFailure::local))
        .transpose()
        .map(|value| value.unwrap_or(default))
}

pub(super) fn new_id(prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{}-{millis}", process::id())
}

pub(super) fn exit_code_for_class(class: &ErrorClass) -> i32 {
    match class {
        ErrorClass::InvalidInput => 2,
        ErrorClass::Capability => 3,
        ErrorClass::Conflict => 4,
        ErrorClass::Forbidden | ErrorClass::Authentication => 5,
        ErrorClass::Unresolved => 6,
        ErrorClass::Unavailable => 7,
        ErrorClass::Budget => 8,
        ErrorClass::Store => 9,
        ErrorClass::Replay => 10,
    }
}

pub(super) fn exit_code_for_http_status(status: u16) -> i32 {
    match status {
        401 | 403 => 5,
        409 => 4,
        422 => 10,
        503 => 7,
        _ => 2,
    }
}

pub(super) fn readable_summary(value: &Value) -> String {
    if let Some(run_id) = value.get("workflow_run_id").and_then(Value::as_str) {
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("accepted");
        return format!("workflow run {run_id}: {status}\n");
    }
    if let Some(run) = value.get("run") {
        let run_id = run
            .get("workflow_run_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let status = run
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return format!("workflow run {run_id}: {status}\n");
    }
    if let Some(valid) = value.get("valid").and_then(Value::as_bool) {
        return format!("definition valid: {valid}\n");
    }
    if let Some(events) = value.get("events").and_then(Value::as_array) {
        return format!("events returned: {}\n", events.len());
    }
    if let Some(matched) = value.get("matched").and_then(Value::as_bool) {
        return format!("offline replay matched: {matched}\n");
    }
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned())
}

pub(super) fn usage() -> String {
    "sts2-workflow commands:\n  serve --listen <loopback> --store <path> --auth-profile <name>\n  validate <definition.json> --capabilities <manifest.json>\n  inspect <definition.json>\n  diff <old.json> <new.json>\n  run <definition.json> --instance <id> --profile <name>\n  status <run-id>\n  events <run-id> --after-sequence <n> --limit <n>\n  pause|resume|step|cancel <run-id> --expected-revision <n>\n  replay <run-id> --offline\n  export <run-id> --redacted --output <approved-path>\n\nClient commands read STS2_WORKFLOW_TOKEN_<AUTH_PROFILE>; default profile is default.\n"
        .to_owned()
}
