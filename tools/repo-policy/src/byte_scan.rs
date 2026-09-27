// SPDX-License-Identifier: MIT

//! Encoding-independent primitives for the purely structural rules.
//!
//! These exist because `fs::read_to_string` fails for a *whole file* on any
//! invalid byte, and a rule that skips an undecodable file therefore exempts
//! that file from itself. A tracked text file carrying one stray byte was
//! consequently exempt from every rule that read it that way. Bytes have no
//! decode step, so there is no failure mode here left to skip.

use std::path::Path;

/// Reads `path` as raw bytes, or `None` if it cannot be opened or read.
///
/// The `None` case is a file that cannot be read at all, which is rare and is
/// reported by the caller when it has a rule id to report it under.
pub(crate) fn read_bytes(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// Counts the nonblank lines in `bytes`, byte-wise.
///
/// "Blank" is decided on ASCII whitespace only, the same set the decoded
/// `str::lines` count used, so a count does not change by switching to bytes.
pub(crate) fn nonblank_line_count(bytes: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
        .count()
}

#[cfg(test)]
mod tests {
    use super::nonblank_line_count;

    /// The count must not depend on the file being decodable, which is the
    /// whole point: the decoded form of this same input yields nothing.
    #[test]
    fn counts_nonblank_lines_in_a_file_that_is_not_valid_utf8() {
        assert_eq!(nonblank_line_count(b"a\n\n\xff\nb\n\n"), 3);
    }

    /// And it must agree with the decoded count on files that do decode, so
    /// switching representation did not move any existing number.
    #[test]
    fn agrees_with_the_decoded_count_on_valid_utf8() {
        for text in ["", "\n", "a\n", "a\n\nb\n", "   \n\t\nx\n"] {
            let decoded = text.lines().filter(|line| !line.trim().is_empty()).count();
            assert_eq!(nonblank_line_count(text.as_bytes()), decoded, "{text:?}");
        }
    }
}
