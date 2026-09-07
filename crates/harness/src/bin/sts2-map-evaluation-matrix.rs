// SPDX-License-Identifier: MIT

use std::error::Error;
use sts2_harness::run_bounded_synthetic_context_matrix;

fn main() -> Result<(), Box<dyn Error>> {
    let report = run_bounded_synthetic_context_matrix()?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
