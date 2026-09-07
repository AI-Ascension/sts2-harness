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
    let terminal = v3_state(
        "state_response",
        "4",
        state_id,
        terminal_generation,
        "victory",
        json!([]),
    )?;
    let reobserve = format!(
        "{}{}",
        reply_artifact(3, reobserved.clone()),
        reply_artifact(4, terminal.clone())
    );
    let normal_failure = if expert_profile
        && matches!(failure, "expert-eof" | "expert-timeout" | "expert-gateway")
    {
        "success"
    } else {
        failure
    };
    let first_normal = match normal_failure {
        "success" if expert_profile && failure == "expert-gateway" => format!(
            "{}{}{}{}{}{}",
            init_sequence(false),
            reply_artifact(1, initial.clone()),
            reply_artifact(2, initial_catalog.clone()),
            reply_artifact(3, reobserved.clone()),
            reply_artifact(4, terminal.clone()),
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
    let second_normal = format!(
        "{}{}{}",
        init_sequence(false),
        reply_artifact(3, reobserved),
        reply_artifact(4, terminal)
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
                "{}{}{}{}{}{}",
                init_sequence(true),
                reply_artifact(1, initial_expert),
                reply(json!({
                    "jsonrpc":"2.0",
                    "id":2,
                    "result":{"isError":true,"content":[{"type":"text","text":"gateway error -32008: gateway request timed out"}]}
                })),
                reply_artifact(3, terminal_expert.clone()),
                reply_artifact(4, terminal_expert.clone()),
                keep_reading()
            ),
            _ => format!(
                "{}{}{}",
                init_sequence(true),
                reply_artifact(1, initial_expert),
                keep_reading()
            ),
        };
        let (reobserve_id, observe_id) = if matches!(failure, "expert-eof" | "expert-timeout") {
            (3, 4)
        } else {
            (2, 3)
        };
        let second_expert = format!(
            "{}{}{}",
            init_sequence(true),
            reply_artifact(reobserve_id, terminal_expert.clone()),
            reply_artifact(observe_id, terminal_expert)
        );
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
