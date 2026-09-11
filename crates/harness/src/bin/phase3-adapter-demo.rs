// SPDX-License-Identifier: MIT

fn main() {
    if let Err(error) = sts2_harness::phase3_adapter_demo::run() {
        eprintln!("phase3 adapter demo: {error}");
        std::process::exit(1);
    }
}
