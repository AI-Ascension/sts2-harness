// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn map_input_bound_is_wider_but_ordinary_input_bound_stays_128k() {
    assert_eq!(
        request_limit(&json!({"schema": "sts2.exo-decision-v1"})),
        OUTPUT_LIMIT
    );
    assert_eq!(
        request_limit(&json!({
            "schema": "sts2.exo-decision-map-v1",
            "map_context": {}
        })),
        INPUT_LIMIT
    );
}

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
    let captured = capture_stream(
        std::io::Cursor::new(vec![b'x'; OUTPUT_LIMIT + 17]),
        OUTPUT_LIMIT,
    )
    .join()
    .map_err(|_| "capture reader panicked")?;
    assert_eq!(captured.bytes.len(), OUTPUT_LIMIT);
    assert_eq!(captured.total_bytes, OUTPUT_LIMIT + 17);
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
