// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn model_output_must_be_a_bounded_catalog_choice() {
    let ids = vec![json!("end:1")];
    assert!(validate(r#"{"action_ids":["end:1"],"rationale":"No energy"}"#, &ids).is_ok());
    assert!(validate(r#"{"action_ids":["other"],"rationale":"No energy"}"#, &ids).is_err());
    assert!(
        validate(
            r#"{"action_ids":["end:1","end:1"],"rationale":"No energy"}"#,
            &ids
        )
        .is_err()
    );
    assert!(
        validate(
            r#"{"action_ids":["end:1"],"rationale":"No energy","tool":"shell"}"#,
            &ids
        )
        .is_err()
    );
    assert!(validate(&"x".repeat(8193), &ids).is_err());
}

#[test]
fn provider_capture_is_bounded_and_keeps_raw_stream_out_of_the_record()
-> Result<(), Box<dyn std::error::Error>> {
    let captured = capture_stream(std::io::Cursor::new(vec![b'x'; LIMIT + 17]), LIMIT)
        .join()
        .map_err(|_| "capture reader panicked")?;
    assert_eq!(captured.bytes.len(), LIMIT);
    assert_eq!(captured.total_bytes, LIMIT + 17);
    assert!(captured.truncated);

    let events = parse_codex_events(
        br#"{"type":"thread.started","thread_id":"thread-1"}
{"type":"turn.completed","usage":{"input_tokens":2,"cached_input_tokens":1,"output_tokens":3}}"#,
    )?;
    let record = accounting_record(
        &json!({"model_execution_id":"model-7","private_prompt":"do not retain"}),
        b"private prompt",
        &events,
        &ProviderExecution {
            completed: true,
            input_written: true,
            stdout_invalid: false,
            stderr_invalid: false,
            stdout_bytes: 100,
            stderr_bytes: 0,
        },
        Some("decision-digest"),
        true,
    );
    let serialized = serde_json::to_string(&record)?;
    assert_eq!(record["usage_status"], "reported");
    assert_eq!(record["provider_request_id"], "thread-1");
    assert!(!serialized.contains("private prompt"));
    assert!(!serialized.contains("do not retain"));
    Ok(())
}

#[test]
fn prepared_astra_input_lists_exact_stdin_schema_and_configuration() {
    let mut capture = sts2_harness::MemoryCapture::new(sts2_harness::CaptureMode::Memory, 8, LIMIT)
        .expect("capture config");
    PreparedAstraInput::new(b"stdin", br#"{"type":"object"}"#, b"argv").capture(
        &mut capture,
        "execution-7",
        Some("attempt-2"),
    );
    let records = capture.records().collect::<Vec<_>>();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].component_kind.unwrap().as_str(), "stdin");
    assert_eq!(records[1].component_kind.unwrap().as_str(), "output_schema");
    assert_eq!(records[2].component_kind.unwrap().as_str(), "configuration");
    assert_eq!(records[0].content.as_deref(), Some(b"stdin".as_slice()));
    assert_eq!(records[0].attempt_id.as_deref(), Some("attempt-2"));
}

#[cfg(unix)]
#[test]
fn actual_fake_codex_receives_the_same_prepared_components_as_capture()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let provider_directory = Temporary::create()?;
    let oracle_directory = Temporary::create()?;
    let observed_directory = oracle_directory.0.to_string_lossy().into_owned();
    if observed_directory.contains('\'') {
        return Err("temporary path contains an unsupported quote".into());
    }
    let fake = oracle_directory.0.join("codex-fake");
    let script = format!(
        "#!/bin/sh\nset -eu\nobs='{observed_directory}'\nmkdir -p \"$obs\"\ncat > \"$obs/stdin.bin\"\nschema=''\noutput=''\nprevious=''\nfor arg in \"$@\"; do\n  if [ \"$previous\" = '--output-schema' ]; then schema=\"$arg\"; fi\n  if [ \"$previous\" = '--output-last-message' ]; then output=\"$arg\"; fi\n  previous=\"$arg\"\ndone\ncat \"$schema\" > \"$obs/schema.json\"\nprintf '%s\\n' \"$@\" > \"$obs/argv.txt\"\nprintf '%s' '{{\"action_ids\":[\"combat.end-turn\"],\"rationale\":\"synthetic oracle\"}}' > \"$output\"\nprintf '%s\\n%s\\n' '{{\"type\":\"thread.started\",\"thread_id\":\"oracle-thread-1\"}}' '{{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":17,\"cached_input_tokens\":2,\"output_tokens\":5}}}}'\n"
    );
    std::fs::write(&fake, script)?;
    let mut permissions = std::fs::metadata(&fake)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&fake, permissions)?;

    let request = json!({
        "model_execution_id":"oracle-execution-1",
        "legal_action_ids":["combat.end-turn"],
    });
    let request_bytes = serde_json::to_vec(&request)?;
    let ids = request["legal_action_ids"]
        .as_array()
        .ok_or("missing test action catalog")?;
    let mut capture =
        sts2_harness::MemoryCapture::new(sts2_harness::CaptureMode::Memory, 8, LIMIT)?;
    let decision = decide_with_executable(
        &request,
        &request_bytes,
        ids,
        &provider_directory,
        &mut capture,
        fake.to_str().ok_or("fake path is not UTF-8")?,
    )?;
    assert_eq!(decision["action_ids"][0], "combat.end-turn");

    let records = capture
        .records()
        .filter(|record| record.component_kind.is_some())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 3);
    assert_eq!(
        records[0].content.as_deref(),
        Some(std::fs::read(oracle_directory.0.join("stdin.bin"))?.as_slice())
    );
    assert_eq!(
        records[1].content.as_deref(),
        Some(std::fs::read(oracle_directory.0.join("schema.json"))?.as_slice())
    );
    let configuration: Value = serde_json::from_slice(
        records[2]
            .content
            .as_deref()
            .ok_or("configuration content is absent")?,
    )?;
    let configured = configuration["argv"]
        .as_array()
        .ok_or("captured argv is absent")?
        .iter()
        .map(|value| value.as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    let fake_path = fake.to_string_lossy();
    let fake_index = configured
        .iter()
        .position(|argument| argument == fake_path.as_ref())
        .ok_or("fake executable is absent from captured argv")?;
    let child_args = configured[(fake_index + 1)..].join("\n") + "\n";
    assert_eq!(
        child_args,
        std::fs::read_to_string(oracle_directory.0.join("argv.txt"))?
    );
    assert_eq!(
        configuration["working_directory"],
        provider_directory.0.to_string_lossy().as_ref()
    );

    std::fs::remove_dir_all(provider_directory.0)?;
    std::fs::remove_dir_all(oracle_directory.0)?;
    Ok(())
}
