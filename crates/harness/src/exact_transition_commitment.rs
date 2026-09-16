// SPDX-License-Identifier: MIT

/// Version bound into every transition commitment.
pub const TRANSITION_COMMITMENT_VERSION: &str = "asc-transition:v1";
/// Serialized prefix of a transition commitment.
pub const TRANSITION_COMMITMENT_PREFIX: &str = "asc-transition:v1:sha256:";
/// Domain separator for the transition commitment.
pub const TRANSITION_DOMAIN: &[u8] = b"AI-ASCENSION/TRANSITION/v1\0";
pub const MAX_TRANSITION_RECORDS: usize = 100_000;
pub const MAX_TRANSITION_LABEL_BYTES: usize = 256;

fn valid_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TRANSITION_LABEL_BYTES && !value.contains('\0')
}

fn valid_commitment(value: &str) -> Option<()> {
    let hex = value.strip_prefix(TRANSITION_COMMITMENT_PREFIX)?;
    let lowercase = hex
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    (hex.len() == 64 && lowercase).then_some(())
}
