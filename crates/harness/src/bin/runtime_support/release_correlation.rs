// SPDX-License-Identifier: MIT

/// Each release call has a fresh transport identity, independent of its lease.
pub(crate) fn release_correlation() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn release_requests_have_distinct_canonical_uuid4_identities() {
        let first = super::release_correlation();
        let second = super::release_correlation();
        assert_ne!(first, second);
        for value in [first, second] {
            let parsed = uuid::Uuid::parse_str(&value).expect("UUID");
            assert_eq!(parsed.get_version_num(), 4);
            assert_eq!(parsed.get_variant(), uuid::Variant::RFC4122);
            assert_eq!(parsed.to_string(), value);
        }
    }
}
