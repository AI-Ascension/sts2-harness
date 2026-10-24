// SPDX-License-Identifier: MIT

use std::path::PathBuf;
use std::sync::Arc;

use sts2_harness::management::{FileSeedDerivationKeyAuthority, SeedDerivationKeyAuthority};

const KEYRING_PATH_ENV: &str = "STS2_SEED_DERIVATION_KEYRING_PATH";
const MAX_KEYRING_PATH_BYTES: usize = 1024;

/// Loads a startup-pinned keyring only from the protected-file adapter. The
/// environment variable names a path; raw key material is never accepted in
/// process configuration.
pub(super) fn authority_from_environment()
-> Result<Option<Arc<dyn SeedDerivationKeyAuthority>>, String> {
    let Some(path) = std::env::var_os(KEYRING_PATH_ENV) else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    if !path.is_absolute() || path.as_os_str().len() > MAX_KEYRING_PATH_BYTES {
        return Err(format!(
            "{KEYRING_PATH_ENV} must name a bounded absolute keyring path"
        ));
    }
    let authority = FileSeedDerivationKeyAuthority::open(&path)
        .map_err(|error| format!("{KEYRING_PATH_ENV} could not be loaded ({error})"))?;
    Ok(Some(Arc::new(authority)))
}
