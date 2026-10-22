// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;

#[test]
fn ignores_unrelated_dispositions_but_refuses_auto_reaped_children() {
    assert!(inspect_status("Name:\ttest\nSigIgn:\t0000000000000000\n").is_ok());
    let unrelated = 1_u64 << (Signal::PIPE.as_raw() - 1);
    assert!(inspect_status(&format!("SigIgn:\t{unrelated:016x}\n")).is_ok());
    let ignored = unrelated | CHILD_SIGNAL_BIT;
    let error = inspect_status(&format!("SigIgn:\t{ignored:016x}\n"))
        .expect_err("ignored SIGCHLD must be refused before any child is spawned");
    assert_eq!(error.code, "owned_management_child_wait");
    assert!(error.message.contains("SIGCHLD is ignored"));
}

#[test]
fn unknown_or_ambiguous_signal_state_fails_closed() {
    for status in [
        "Name:\ttest\n",
        "SigIgn:\n",
        "SigIgn:\t000000000000000g\n",
        "SigIgn:\t+000000000000000\n",
        "SigIgn:\t10000000000000000\n",
        "SigIgn:\t0\nSigIgn:\t0\n",
    ] {
        assert_eq!(
            inspect_status(status)
                .expect_err("unverified disposition")
                .code,
            "owned_management_child_wait",
        );
    }
}

#[test]
fn status_inspection_has_a_fixed_byte_bound() {
    let mut status = String::from("SigIgn:\t0000000000000000\n");
    status.push_str(&"x".repeat(MAX_STATUS_BYTES - status.len()));
    assert!(inspect_status(&status).is_ok());
    status.push('x');
    assert_eq!(
        inspect_status(&status)
            .expect_err("oversized proc status")
            .code,
        "owned_management_child_wait",
    );
}
