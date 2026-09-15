// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn persisted_collection_visitor_does_not_decode_one_over_limit_element() {
    static DECODED: AtomicUsize = AtomicUsize::new(0);
    struct Probe;
    impl<'de> Deserialize<'de> for Probe {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            DECODED.fetch_add(1, Ordering::SeqCst);
            u8::deserialize(d)?;
            Ok(Self)
        }
    }
    let bytes = serde_json::to_vec(&vec![0_u8; MAX_POLICY_VERSIONS + 1]).unwrap();
    let mut decoder = serde_json::Deserializer::from_slice(&bytes);
    assert!(bounded_vec::<_, Probe, MAX_POLICY_VERSIONS>(&mut decoder).is_err());
    assert_eq!(DECODED.load(Ordering::SeqCst), MAX_POLICY_VERSIONS);
}
