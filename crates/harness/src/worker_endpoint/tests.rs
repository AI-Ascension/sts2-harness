// SPDX-License-Identifier: MIT

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        fn valid_payload() -> Vec<u8> {
            serde_json::to_vec(&json!({
                "version": 1,
                "launch_nonce": "12345678-1234-4234-8234-123456789abc",
                "watchdog_boot_id": "22345678-1234-4234-8234-123456789abc",
                "component_id": "harness",
                "expected_peer": {
                    "platform": "linux",
                    "pid": 1,
                    "creation_token": "1",
                    "executable": "/bin/true",
                    "executable_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "uid": 0,
                    "gid": 0
                }
            }))
            .unwrap_or_default()
        }

        #[test]
        fn bootstrap_decoder_rejects_unknown_and_duplicate_fields() {
            assert!(parse_bootstrap_payload(&valid_payload()).is_ok());
            let unknown = br#"{"version":1,"launch_nonce":"12345678-1234-4234-8234-123456789abc","watchdog_boot_id":"22345678-1234-4234-8234-123456789abc","component_id":"harness","expected_peer":{},"extra":1}"#;
            assert!(parse_bootstrap_payload(unknown).is_err());
            let duplicate = br#"{"version":1,"version":1,"launch_nonce":"12345678-1234-4234-8234-123456789abc","watchdog_boot_id":"22345678-1234-4234-8234-123456789abc","component_id":"harness","expected_peer":{}}"#;
            assert!(parse_bootstrap_payload(duplicate).is_err());
        }

        #[test]
        fn constant_time_comparison_requires_exact_bytes() {
            assert!(constant_time_equal(b"worker", b"worker"));
            assert!(!constant_time_equal(b"worker", b"worker2"));
            assert!(!constant_time_equal(b"worker", b"workEr"));
        }
    }
