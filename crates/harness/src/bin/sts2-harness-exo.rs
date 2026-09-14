// SPDX-License-Identifier: MIT

//! Offline configuration probe for the harness-owned Exo adapter.
//!
//! Network/provider execution is deliberately supplied by an operator-owned transport and is not
//! performed by this probe. A successful configuration parse is not live Exo evidence.

use sts2_harness::{EXO_SOURCE_REVISION, ExoConfig};

const REVIEWED_EXO_REVISION: &str = EXO_SOURCE_REVISION;

fn main() {
    let result = ExoConfig::new(REVIEWED_EXO_REVISION, 128 * 1024, 8 * 1024, 120_000);
    match result {
        Ok(config) => println!(
            "exo adapter configuration shape accepted for revision {}; live connectivity: unverified",
            config.revision
        ),
        Err(error) => println!("exo adapter configuration rejected: {error}"),
    }
}
