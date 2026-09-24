// SPDX-License-Identifier: MIT

use super::SeedBindingError;

/// Inclusive bound on a canonical seed's UTF-8 byte length. It matches the
/// seeded-run consumer bound so an authored seed and a transport seed stay
/// comparable.
pub const MAX_SEED_BYTES: usize = 64;

/// Normalize a seed to its canonical UTF-8 form.
///
/// The canonical form is the exact supplied bytes: non-empty, at most
/// [`MAX_SEED_BYTES`] UTF-8 bytes, free of control characters, and with no
/// leading or trailing whitespace. A multibyte character counts as its encoded
/// byte length, so a 64-byte multibyte seed is accepted and a 65-byte one is not.
///
/// # Errors
///
/// Returns [`SeedBindingError`] naming the first failing bound.
pub fn canonicalize_seed(raw: &str) -> Result<String, SeedBindingError> {
    if raw.is_empty() {
        return Err(SeedBindingError::EmptySeed);
    }
    if raw.len() > MAX_SEED_BYTES {
        return Err(SeedBindingError::SeedTooLarge);
    }
    if raw.chars().any(char::is_control) {
        return Err(SeedBindingError::SeedControlCharacter);
    }
    if raw.trim() != raw {
        return Err(SeedBindingError::SeedNotCanonical);
    }
    Ok(raw.to_owned())
}
