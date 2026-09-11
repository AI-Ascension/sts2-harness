// SPDX-License-Identifier: MIT

fn fake_mcp_script(fixture: &Fixture) -> Result<String, Box<dyn std::error::Error>> {
    let (normal_responses, expert_responses) = rest_response_sets()?;
    fake_mcp_script_with_sets(fixture, normal_responses, expert_responses)
}

fn fake_mcp_script_with_sets(
    fixture: &Fixture,
    normal_responses: Vec<Value>,
    expert_responses: Vec<Value>,
) -> Result<String, Box<dyn std::error::Error>> {
    let directory = fixture.shell_path()?;
    let normal = profile_script(
        &directory,
        "normal.requests",
        "runtime-v3-gameplay-mcp",
        normal_tools(),
        normal_responses,
    );
    let expert = profile_script(
        &directory,
        "expert.requests",
        "runtime-v4-expert-rest-action-mcp",
        rest_tools(),
        expert_responses,
    );
    Ok(format!(
        "if [ \"$STS2_RUNTIME_PROFILE\" = \"runtime-v4-expert-rest-action\" ]; then\n{expert}else\n{normal}fi\n"
    ))
}
