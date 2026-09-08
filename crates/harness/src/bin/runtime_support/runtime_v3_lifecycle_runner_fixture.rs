// SPDX-License-Identifier: MIT

fn runner_script(
    fixture: &Fixture,
    failure: &str,
    expert_profile: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let (state_id, generation, terminal_generation) = if expert_profile {
        ("live:7", 7, 8)
    } else {
        ("combat-1", 0, 1)
    };
    let actions = json!([
        {"action_id":"end:7", "action":{"kind":"end_turn"}}
    ]);
    let initial = v3_state(
        "state_response",
        "1",
        state_id,
        generation,
        "combat",
        actions,
    )?;
    let mut initial_catalog = initial.clone();
    initial_catalog["kind"] = json!("legal_actions_response");
    initial_catalog["correlation_id"] = json!("2");
    initial_catalog["observation"] = Value::Null;
    let reobserved = v3_state(
        "reobserve_response",
        "3",
        state_id,
        terminal_generation,
        "victory",
        json!([]),
    )?;
    let reobserve = reply_artifact(3, reobserved.clone());
    let normal_failure = if expert_profile
        && matches!(
            failure,
            "expert-eof"
                | "expert-timeout"
                | "expert-gateway"
                | "expert-reobserve-eof"
                | "expert-reobserve-timeout"
                | "expert-reobserve-gateway"
                | "expert-reobserve-schema"
                | "expert-reobserve-auth"
                | "expert-reobserve-identity"
        )
    {
        if failure.starts_with("expert-reobserve-") {
            "rpc"
        } else {
            "success"
        }
    } else {
        failure
    };
    let first_normal = match normal_failure {
        "success" if expert_profile && failure == "expert-gateway" => format!(
            "{}{}{}{}{}",
            init_sequence(false),
            reply_artifact(1, initial.clone()),
            reply_artifact(2, initial_catalog.clone()),
            reply_artifact(3, reobserved.clone()),
            keep_reading()
        ),
        "success" => format!(
            "{}{}{}{}",
            init_sequence(false),
            reply_artifact(1, initial.clone()),
            reply_artifact(2, initial_catalog),
            keep_reading()
        ),
        "tool" => format!(
            "{}{}{}{}{}",
            init_sequence(false),
            reply_artifact(1, initial.clone()),
            reply(json!({
                "jsonrpc":"2.0",
                "id":2,
                "result":{"isError":true,"content":[{"type":"text","text":"gateway error -32008: gateway request timed out"}]}
            })),
            reobserve.clone(),
            keep_reading()
        ),
        "rpc" => format!(
            "{}{}{}{}{}",
            init_sequence(false),
            reply_artifact(1, initial.clone()),
            reply(json!({
                "jsonrpc":"2.0",
                "id":2,
                "error":{"code":-32008,"message":"gateway request timed out"}
            })),
            reobserve.clone(),
            keep_reading()
        ),
        "timeout" => format!(
            "{}{}{}",
            init_sequence(false),
            reply_artifact(1, initial),
            pipe_timeout()
        ),
        _ => format!(
            "{}{}{}",
            init_sequence(false),
            reply_artifact(1, initial),
            pipe_eof()
        ),
    };
    let mut reobserved_id4 = reobserved.clone();
    reobserved_id4["correlation_id"] = json!("4");
    let second_normal = format!(
        "{}{}",
        init_sequence(false),
        reply_for_ids(&[(3, reobserved.clone()), (4, reobserved_id4)])
    );
    let normal_branch = format!(
        "if [ -e normal-started ]; then\n{}{}else\n: > normal-started\n{}fi\n",
        second_normal,
        keep_reading(),
        first_normal
    );
    let script = if expert_profile {
        let initial_expert = expert_state(state_id, generation, "combat", false)?;
        let terminal_expert = expert_state(state_id, terminal_generation, "victory", true)?;
        let first_expert = match failure {
            "expert-eof" => format!(
                "{}{}{}",
                init_sequence(true),
                reply_artifact(1, initial_expert),
                pipe_eof()
            ),
            "expert-timeout" => format!(
                "{}{}{}",
                init_sequence(true),
                reply_artifact(1, initial_expert),
                pipe_timeout()
            ),
            "expert-gateway" => format!(
                "{}{}{}{}{}",
                init_sequence(true),
                reply_artifact(1, initial_expert),
                reply(json!({
                    "jsonrpc":"2.0",
                    "id":2,
                    "result":{"isError":true,"content":[{"type":"text","text":"gateway error -32008: gateway request timed out"}]}
                })),
                reply_artifact(3, terminal_expert.clone()),
                keep_reading()
            ),
            "expert-reobserve-eof" => format!(
                "{}{}: > expert-reobserve-failed\n{}",
                init_sequence(true),
                reply_artifact(1, initial_expert),
                pipe_eof()
            ),
            "expert-reobserve-timeout" => format!(
                "{}{}: > expert-reobserve-failed\n{}",
                init_sequence(true),
                reply_artifact(1, initial_expert),
                pipe_timeout()
            ),
            "expert-reobserve-gateway" => format!(
                "{}{}{}",
                init_sequence(true),
                reply_artifact(1, initial_expert),
                reply(json!({
                    "jsonrpc":"2.0",
                    "id":2,
                    "error":{"code":-32008,"message":"gateway request timed out"}
                }))
            ),
            "expert-reobserve-schema" => {
                let mut invalid = terminal_expert.clone();
                invalid["state_id"] = Value::Null;
                format!(
                    "{}{}{}{}",
                    init_sequence(true),
                    reply_artifact(1, initial_expert),
                    reply_artifact(2, invalid),
                    keep_reading()
                )
            }
            "expert-reobserve-auth" => format!(
                "{}{}{}{}",
                init_sequence(true),
                reply_artifact(1, initial_expert),
                reply(json!({
                    "jsonrpc":"2.0",
                    "id":2,
                    "error":{"code":-32001,"message":"unauthorized"}
                })),
                keep_reading()
            ),
            "expert-reobserve-identity" => {
                let mismatched = expert_state("different-live", terminal_generation, "victory", true)?;
                format!(
                    "{}{}{}{}",
                    init_sequence(true),
                    reply_artifact(1, initial_expert),
                    reply_artifact(2, mismatched),
                    keep_reading()
                )
            }
            _ => format!(
                "{}{}{}",
                init_sequence(true),
                reply_artifact(1, initial_expert),
                keep_reading()
            ),
        };
        let reobserve_id = if matches!(failure, "expert-eof" | "expert-timeout") {
            3
        } else {
            2
        };
        let second_expert = if matches!(
            failure,
            "expert-reobserve-eof"
                | "expert-reobserve-timeout"
                | "expert-reobserve-gateway"
                | "expert-reobserve-schema"
                | "expert-reobserve-auth"
            | "expert-reobserve-identity"
        ) {
            let retry_response = reply_artifact(3, terminal_expert.clone());
            if matches!(failure, "expert-reobserve-eof" | "expert-reobserve-timeout") {
                let first_response = if failure == "expert-reobserve-eof" {
                    pipe_eof()
                } else {
                    pipe_timeout()
                };
                format!(
                    "if [ -e expert-reobserve-failed ]; then\n: > expert-reobserve-succeeded\n{}{}{}else\n: > expert-reobserve-failed\n{}{}fi\n",
                    init_sequence(true),
                    retry_response,
                    keep_reading(),
                    init_sequence(true),
                    first_response
                )
            } else if failure == "expert-reobserve-gateway" {
                format!("{}{}", init_sequence(true), reply_if_requested(3, terminal_expert.clone()))
            } else {
                format!("{}{}{}", init_sequence(true), retry_response, keep_reading())
            }
        } else {
            format!(
                "{}{}{}",
                init_sequence(true),
                reply_artifact(reobserve_id, terminal_expert.clone()),
                keep_reading()
            )
        };
        format!(
            "cd '{}' || exit 1\nif [ \"$STS2_RUNTIME_PROFILE\" = \"runtime-v4-expert\" ]; then\nif [ -e expert-started ]; then\n{}else\n: > expert-started\n{}fi\nelse\n{}fi\n",
            fixture.0.display(),
            second_expert,
            first_expert,
            normal_branch
        )
    } else {
        format!("cd '{}' || exit 1\n{}", fixture.0.display(), normal_branch)
    };
    fixture.script(&script)
}
