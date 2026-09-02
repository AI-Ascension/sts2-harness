// SPDX-License-Identifier: MIT

#[path = "runtime_support/mod.rs"]
mod runtime_support;

fn main() {
    match runtime_support::RuntimeConfig::from_environment().and_then(runtime_support::run) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("sts2-harness runtime failed: {error}");
            std::process::exit(2);
        }
    }
}
