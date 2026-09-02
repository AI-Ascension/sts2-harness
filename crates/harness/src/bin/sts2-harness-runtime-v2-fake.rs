// SPDX-License-Identifier: MIT

use sts2_harness::run_runtime_v2_fake_trace;

fn main() {
    match run_runtime_v2_fake_trace() {
        Ok(report) => print!("{}", report.trace_bytes()),
        Err(error) => {
            eprintln!("sts2-harness Runtime-v2 fake failed: {error}");
            std::process::exit(2);
        }
    }
}
