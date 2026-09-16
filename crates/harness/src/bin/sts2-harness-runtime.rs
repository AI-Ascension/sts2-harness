// SPDX-License-Identifier: MIT

#[path = "runtime_support/mod.rs"]
mod runtime_support;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("serve-workflow") {
        match runtime_support::serve_workflow() {
            Ok(()) => return,
            Err(error) => {
                eprintln!("sts2-harness live workflow service failed: {error}");
                std::process::exit(2);
            }
        }
    }
    if std::env::var_os("STS2_WORKER_ENDPOINT_NAMESPACE").is_some() {
        match sts2_harness::worker_endpoint::run_from_environment() {
            Ok(()) => return,
            Err(error) => {
                eprintln!("sts2-harness worker endpoint failed: {error}");
                std::process::exit(2);
            }
        }
    }
    match runtime_support::RuntimeConfig::from_environment().and_then(runtime_support::run) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("sts2-harness runtime failed: {error}");
            std::process::exit(2);
        }
    }
}
