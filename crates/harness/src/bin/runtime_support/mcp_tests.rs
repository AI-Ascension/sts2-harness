// SPDX-License-Identifier: MIT

use super::*;
#[test]
fn raw_mcp_failures_and_unknown_observation_fields_are_not_logged() {
    let response = json!({"result":{"isError":true,"content":[{"text":"secret-marker"}]}});
    assert_eq!(
        require_success(&response, "get_state"),
        Err("get_state returned an MCP error".into())
    );
}
