// SPDX-License-Identifier: MIT

fn main() {
    sts2_harness::management::run_cli(std::env::args().skip(1).collect());
}
