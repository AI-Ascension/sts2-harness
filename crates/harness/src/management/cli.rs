// SPDX-License-Identifier: MIT

use std::env;
use std::process;
use std::sync::Arc;

use self::support::{
    CliFailure, CliOutput, client_for, new_id, parse_address, parse_options, parse_u64_option,
    read_json_file, request_and_render, required_option, response_failure, usage,
    validate_output_path,
};
use super::{
    CommandKind, CommandParameters, CommandRequest, DiffRequest, EnvironmentAuthenticator,
    ExportRequest, ExportResponse, FileWorkflowStore, InspectRequest, MANAGEMENT_SCHEMA_VERSION,
    ManagementReplayRequest, ManagementServer, OutputFormat, RunRequest, ServerConfig,
    ValidateRequest, validate_identifier,
};

const DEFAULT_LISTEN: &str = "127.0.0.1:8787";

#[path = "cli_control.rs"]
mod control;
#[path = "cli_support.rs"]
mod support;

pub fn run_cli(args: Vec<String>) {
    let result = run(args);
    match result {
        Ok(output) => {
            if let Some(bytes) = output.0 {
                println!("{}", String::from_utf8_lossy(&bytes));
            }
        }
        Err(error) => {
            if let Some(bytes) = error.json {
                eprintln!("{}", String::from_utf8_lossy(&bytes));
            } else {
                eprintln!("{}", error.message);
            }
            process::exit(error.exit_code);
        }
    }
}

fn run(args: Vec<String>) -> Result<CliOutput, CliFailure> {
    let command = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| CliFailure::invalid(usage()))?;
    if matches!(command, "help" | "--help" | "-h") {
        return Ok(CliOutput(Some(usage().into_bytes())));
    }
    match command {
        "serve" => serve(&args[1..]),
        "validate" => validate_command(&args[1..]),
        "inspect" => inspect_command(&args[1..]),
        "diff" => diff_command(&args[1..]),
        "run" => run_command(&args[1..]),
        "status" => status_command(&args[1..]),
        "events" => events_command(&args[1..]),
        "pause" => control_command(&args[1..], CommandKind::Pause),
        "resume" => control_command(&args[1..], CommandKind::Resume),
        "step" => control_command(&args[1..], CommandKind::Step),
        "cancel" => control_command(&args[1..], CommandKind::Cancel),
        "replay" => control::replay_command(&args[1..]),
        "export" => control::export_command(&args[1..]),
        _ => Err(CliFailure::invalid(format!(
            "unknown command {command}\n{}",
            usage()
        ))),
    }
}

fn serve(args: &[String]) -> Result<CliOutput, CliFailure> {
    let (positionals, options, flags) =
        parse_options(args, &[], &["listen", "store", "auth-profile"], &[])?;
    if !positionals.is_empty() || !flags.is_empty() {
        return Err(CliFailure::invalid(
            "serve does not accept positional arguments or flags",
        ));
    }
    let listen = parse_address(options.get("listen"), DEFAULT_LISTEN)?;
    let store_path = required_option(&options, "store")?;
    let auth_profile = required_option(&options, "auth-profile")?;
    let authenticator =
        Arc::new(EnvironmentAuthenticator::from_profile(auth_profile).map_err(CliFailure::local)?);
    let store = FileWorkflowStore::open(store_path).map_err(CliFailure::local)?;
    let service = Arc::new(super::synthetic_file_store(store));
    let config = ServerConfig::new(listen, authenticator).map_err(CliFailure::local)?;
    let server = ManagementServer::start(config, service).map_err(CliFailure::local)?;
    eprintln!(
        "sts2-workflow serving authenticated loopback management API at {}",
        server.address()
    );
    server.wait().map_err(CliFailure::local)?;
    Ok(CliOutput(None))
}

fn validate_command(args: &[String]) -> Result<CliOutput, CliFailure> {
    let (positionals, options, flags) = parse_options(
        args,
        &[],
        &["capabilities", "format", "listen", "auth-profile"],
        &[],
    )?;
    if positionals.len() != 1 || !flags.is_empty() {
        return Err(CliFailure::invalid(
            "validate requires <definition.json> and --capabilities <manifest.json>",
        ));
    }
    let definition = read_json_file(&positionals[0])?;
    let capabilities = read_json_file(required_option(&options, "capabilities")?)?;
    let body = serde_json::to_vec(&ValidateRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        definition,
        capabilities,
    })
    .map_err(CliFailure::local)?;
    request_and_render(
        &options,
        "POST",
        "/v1/workflow-definitions/validate",
        Some(body),
        client_for(&options)?,
    )
}

fn inspect_command(args: &[String]) -> Result<CliOutput, CliFailure> {
    let (positionals, options, flags) =
        parse_options(args, &[], &["format", "listen", "auth-profile"], &[])?;
    if positionals.len() != 1 || !flags.is_empty() {
        return Err(CliFailure::invalid("inspect requires <definition.json>"));
    }
    let body = serde_json::to_vec(&InspectRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        definition: read_json_file(&positionals[0])?,
        format: OutputFormat::Json,
    })
    .map_err(CliFailure::local)?;
    request_and_render(
        &options,
        "POST",
        "/v1/workflow-definitions/inspect",
        Some(body),
        client_for(&options)?,
    )
}

fn diff_command(args: &[String]) -> Result<CliOutput, CliFailure> {
    let (positionals, options, flags) =
        parse_options(args, &[], &["format", "listen", "auth-profile"], &[])?;
    if positionals.len() != 2 || !flags.is_empty() {
        return Err(CliFailure::invalid("diff requires <old.json> <new.json>"));
    }
    let body = serde_json::to_vec(&DiffRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        old_definition: read_json_file(&positionals[0])?,
        new_definition: read_json_file(&positionals[1])?,
        format: OutputFormat::Json,
    })
    .map_err(CliFailure::local)?;
    request_and_render(
        &options,
        "POST",
        "/v1/workflow-definitions/diff",
        Some(body),
        client_for(&options)?,
    )
}

fn run_command(args: &[String]) -> Result<CliOutput, CliFailure> {
    let (positionals, options, flags) = parse_options(
        args,
        &[],
        &["instance", "profile", "format", "listen", "auth-profile"],
        &[],
    )?;
    if positionals.len() != 1 || !flags.is_empty() {
        return Err(CliFailure::invalid(
            "run requires <definition.json> --instance <id> --profile <name>",
        ));
    }
    let instance_id = required_option(&options, "instance")?;
    let profile = required_option(&options, "profile")?;
    let body = serde_json::to_vec(&RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: new_id("submission"),
        definition: Some(read_json_file(&positionals[0])?),
        artifact_id: None,
        instance_id: instance_id.to_owned(),
        profile: profile.to_owned(),
    })
    .map_err(CliFailure::local)?;
    request_and_render(
        &options,
        "POST",
        "/v1/workflow-runs",
        Some(body),
        client_for(&options)?,
    )
}

fn status_command(args: &[String]) -> Result<CliOutput, CliFailure> {
    let (positionals, options, flags) =
        parse_options(args, &[], &["format", "listen", "auth-profile"], &[])?;
    if positionals.len() != 1 || !flags.is_empty() {
        return Err(CliFailure::invalid("status requires <run-id>"));
    }
    validate_identifier("run_id", &positionals[0]).map_err(CliFailure::local)?;
    request_and_render(
        &options,
        "GET",
        &format!("/v1/workflow-runs/{}", positionals[0]),
        None,
        client_for(&options)?,
    )
}

fn events_command(args: &[String]) -> Result<CliOutput, CliFailure> {
    let (positionals, options, flags) = parse_options(
        args,
        &[],
        &[
            "after-sequence",
            "limit",
            "format",
            "listen",
            "auth-profile",
        ],
        &[],
    )?;
    if positionals.len() != 1 || !flags.is_empty() {
        return Err(CliFailure::invalid(
            "events requires <run-id> [--after-sequence <n>] [--limit <n>]",
        ));
    }
    validate_identifier("run_id", &positionals[0]).map_err(CliFailure::local)?;
    let after = parse_u64_option(&options, "after-sequence", 0)?;
    let limit = parse_u64_option(&options, "limit", 128)?;
    let path = format!(
        "/v1/workflow-runs/{}/events?after_sequence={after}&limit={limit}",
        positionals[0]
    );
    request_and_render(&options, "GET", &path, None, client_for(&options)?)
}

fn control_command(args: &[String], kind: CommandKind) -> Result<CliOutput, CliFailure> {
    let (positionals, options, flags) = parse_options(
        args,
        &[],
        &["expected-revision", "format", "auth-profile", "listen"],
        &[],
    )?;
    if positionals.len() != 1 || !flags.is_empty() {
        return Err(CliFailure::invalid(
            "control commands require <run-id> --expected-revision <n>",
        ));
    }
    validate_identifier("run_id", &positionals[0]).map_err(CliFailure::local)?;
    let revision = parse_u64_option(&options, "expected-revision", 0)?;
    if revision == 0 {
        return Err(CliFailure::invalid("--expected-revision must be positive"));
    }
    let profile = options
        .get("auth-profile")
        .cloned()
        .or_else(|| env::var("STS2_WORKFLOW_AUTH_PROFILE").ok())
        .unwrap_or_else(|| "default".to_owned());
    let body = serde_json::to_vec(&CommandRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: new_id("command"),
        run_id: positionals[0].clone(),
        expected_revision: revision,
        actor_scope: format!("profile:{profile}"),
        kind,
        parameters: CommandParameters::default(),
    })
    .map_err(CliFailure::local)?;
    request_and_render(
        &options,
        "POST",
        &format!("/v1/workflow-runs/{}/commands", positionals[0]),
        Some(body),
        client_for(&options)?,
    )
}
