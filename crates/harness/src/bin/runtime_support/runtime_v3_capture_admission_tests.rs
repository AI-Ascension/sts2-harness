// SPDX-License-Identifier: MIT

use super::arguments_allowed;

#[test]
fn capture_is_only_a_final_unix_execution_suffix() {
    let path = if cfg!(windows) {
        "C:/providers/transport.exe"
    } else {
        "/opt/transport"
    };
    let directory = if cfg!(windows) {
        "C:/capture"
    } else {
        "/var/lib/jev-capture"
    };
    for tactical in [false, true] {
        for gated in [false, true] {
            let mut args = vec!["--model", "jev-1.13.0", "--transport", path];
            if gated {
                args.extend(["--gate", "20"]);
            }
            if tactical {
                args.push("--tactical");
            }
            let plain = args
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>();
            assert!(arguments_allowed(Some("typesafe-jev"), &plain));
            args.extend(["--audit-dir", directory]);
            let mut captured = args
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>();
            assert_eq!(
                arguments_allowed(Some("typesafe-jev"), &captured),
                cfg!(unix)
            );
            assert!(!arguments_allowed(Some("ollama"), &captured));
            for flag in ["--record", "--describe", "--tactical"] {
                captured.push(flag.to_owned());
                assert!(!arguments_allowed(Some("typesafe-jev"), &captured));
                let _ = captured.pop();
            }
        }
    }
}

#[test]
fn incomplete_reordered_or_relative_capture_flags_are_refused() {
    for args in [
        vec!["--audit-dir"],
        vec!["--audit-dir", "/var/lib/capture"],
        vec![
            "--model",
            "jev-1.13.0",
            "--transport",
            "/opt/transport",
            "--audit-dir",
        ],
        vec![
            "--model",
            "jev-1.13.0",
            "--transport",
            "/opt/transport",
            "--audit-dir",
            "relative",
        ],
        vec![
            "--audit-dir",
            "/var/lib/capture",
            "--model",
            "jev-1.13.0",
            "--transport",
            "/opt/transport",
        ],
    ] {
        assert!(!arguments_allowed(
            Some("typesafe-jev"),
            &args.into_iter().map(str::to_owned).collect::<Vec<_>>()
        ));
    }
}
