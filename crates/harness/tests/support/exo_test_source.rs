// SPDX-License-Identifier: MIT

use std::path::PathBuf;

pub fn pinned_exo_test_source() -> Result<PathBuf, String> {
    let source = std::env::var_os("STS2_EXO_TEST_SOURCE").ok_or_else(|| {
        String::from(
            "STS2_EXO_TEST_SOURCE must point to the clean checkout of the reviewed Exo revision",
        )
    })?;
    let source = PathBuf::from(source);
    if !source.is_absolute() {
        return Err(String::from(
            "STS2_EXO_TEST_SOURCE must be an absolute checkout path",
        ));
    }
    Ok(source)
}
