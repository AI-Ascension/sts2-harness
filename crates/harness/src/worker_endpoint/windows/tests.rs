// SPDX-License-Identifier: MIT

#[test]
fn windows_bootstrap_decoder_accepts_the_closed_peer_contract() {
    let payload = br#"{
        "version":1,
        "launch_nonce":"12345678-1234-4234-8234-123456789abc",
        "watchdog_boot_id":"22345678-1234-4234-8234-123456789abc",
        "component_id":"harness",
        "expected_peer":{
            "platform":"windows",
            "pid":1,
            "creation_token":"1",
            "executable":"C:\\Program Files\\STS2\\worker.exe",
            "executable_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "session_id":1,
            "sid":"S-1-5-21-1-2-3-4"
        }
    }"#;
    assert!(parse_bootstrap_payload(payload).is_ok());
}

#[test]
fn windows_auth_comparison_requires_exact_bytes() {
    assert!(constant_time_equal(b"worker", b"worker"));
    assert!(!constant_time_equal(b"worker", b"worker2"));
    assert!(!constant_time_equal(b"worker", b"workEr"));
}
