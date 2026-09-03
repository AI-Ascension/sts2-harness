// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn bounded_gameplay_projection_rejects_malformed_counts() {
    let projected = observation_counts(
        &json!({"energy":3,"hand_count":"secret-marker", "private":"secret-marker"}),
    );
    assert_eq!(projected, json!({"energy":3}));
    let before = json!({"hand_count":1,"energy":3,"draw_pile_count":4,"discard_pile_count":0,"exhaust_pile_count":0});
    let mut after = before.clone();
    after["hand_count"] = json!("malformed");
    assert!(!play_card_observation_changed(&before, &after));
    after["hand_count"] = json!(0);
    assert!(play_card_observation_changed(&before, &after));
}

#[test]
fn lease_cleanup_requires_authoritative_release_confirmation() {
    assert!(confirm_release(Ok(json!({"status":"released"}))).is_ok());
    for response in [json!({}), json!({"status":"allocated"}), Value::Null] {
        assert_eq!(
            confirm_release(Ok(response)),
            Err("gateway did not confirm lease release".into())
        );
    }
    assert_eq!(
        confirm_release(Err("unavailable".into())),
        Err("unavailable".into())
    );
}

#[test]
fn raw_mcp_failures_and_unknown_observation_fields_are_not_logged() {
    let response = json!({"result":{"isError":true,"content":[{"text":"secret-marker"}]}});
    assert_eq!(
        require_success(&response, "get_state"),
        Err("get_state returned an MCP error".into())
    );
}
