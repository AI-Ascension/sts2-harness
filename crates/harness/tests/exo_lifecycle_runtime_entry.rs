// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used)]

#[path = "support/exo_lifecycle_runtime_entry_support.rs"]
mod support;

#[test]
fn shipped_runtime_cli_selects_and_completes_offline_lifecycle_transport() {
    support::run_offline_lifecycle_entry().expect("offline shipped-runtime lifecycle exchange");
}
