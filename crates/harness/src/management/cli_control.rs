// SPDX-License-Identifier: MIT

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use super::super::decode_strict;
use super::{
    CliFailure, CliOutput, ExportRequest, ExportResponse, MANAGEMENT_SCHEMA_VERSION,
    ManagementReplayRequest, client_for, parse_options, request_and_render, required_option,
    response_failure, validate_identifier, validate_output_path,
};

pub(super) fn replay_command(args: &[String]) -> Result<CliOutput, CliFailure> {
    let (positionals, options, flags) = parse_options(
        args,
        &[],
        &["format", "listen", "auth-profile"],
        &["offline"],
    )?;
    if positionals.len() != 1 || !flags.contains("offline") {
        return Err(CliFailure::invalid("replay requires <run-id> --offline"));
    }
    validate_identifier("run_id", &positionals[0]).map_err(CliFailure::local)?;
    let body = serde_json::to_vec(&ManagementReplayRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        run_id: positionals[0].clone(),
        offline: true,
    })
    .map_err(CliFailure::local)?;
    request_and_render(
        &options,
        "POST",
        &format!("/v1/workflow-runs/{}/replay", positionals[0]),
        Some(body),
        client_for(&options)?,
    )
}

pub(super) fn export_command(args: &[String]) -> Result<CliOutput, CliFailure> {
    let (positionals, options, flags) = parse_options(
        args,
        &[],
        &["output", "format", "auth-profile", "listen"],
        &["redacted"],
    )?;
    if positionals.len() != 1 || !flags.contains("redacted") {
        return Err(CliFailure::invalid(
            "export requires <run-id> --redacted --output <approved-path>",
        ));
    }
    let output = PathBuf::from(required_option(&options, "output")?);
    validate_output_path(&output)?;
    validate_identifier("run_id", &positionals[0]).map_err(CliFailure::local)?;
    let body = serde_json::to_vec(&ExportRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        run_id: positionals[0].clone(),
        redacted: true,
    })
    .map_err(CliFailure::local)?;
    let response = client_for(&options)?
        .request_json(
            "POST",
            &format!("/v1/workflow-runs/{}/export", positionals[0]),
            Some(&body),
        )
        .map_err(CliFailure::local)?;
    if response.status / 100 != 2 {
        return Err(response_failure(response));
    }
    let export: ExportResponse = decode_strict(&response.body).map_err(CliFailure::local)?;
    let bytes = serde_json::to_vec_pretty(&export).map_err(CliFailure::local)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .map_err(CliFailure::local)?;
    file.write_all(&bytes).map_err(CliFailure::local)?;
    if options.get("format").map(String::as_str) == Some("json") {
        Ok(CliOutput(Some(bytes)))
    } else {
        Ok(CliOutput(Some(
            format!("exported {}\n", output.display()).into_bytes(),
        )))
    }
}
