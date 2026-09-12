// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::{
    CatalogError, CatalogIdentity, RAW_CATALOG_PREFIX, RawCatalogDigest, SEMANTIC_CATALOG_PREFIX,
    SemanticCatalogDigest,
};

fn raw(seed: char) -> RawCatalogDigest {
    RawCatalogDigest::parse(&format!(
        "{RAW_CATALOG_PREFIX}{}",
        seed.to_string().repeat(64)
    ))
    .expect("raw digest")
}

fn semantic(seed: char) -> SemanticCatalogDigest {
    SemanticCatalogDigest::parse(&format!(
        "{SEMANTIC_CATALOG_PREFIX}{}",
        seed.to_string().repeat(64)
    ))
    .expect("semantic digest")
}

fn identity() -> CatalogIdentity {
    CatalogIdentity::new(raw('a'), semantic('b'), "fixture.semantic_catalog.v1")
        .expect("identity builds")
}

#[test]
fn namespaces_cannot_be_interchanged() {
    let raw_value = raw('a').as_str().to_owned();
    let semantic_value = semantic('b').as_str().to_owned();
    assert_eq!(
        SemanticCatalogDigest::parse(&raw_value).expect_err("raw rejected as semantic"),
        CatalogError::InvalidSemanticDigest
    );
    assert_eq!(
        RawCatalogDigest::parse(&semantic_value).expect_err("semantic rejected as raw"),
        CatalogError::InvalidRawDigest
    );
    assert!(RawCatalogDigest::parse("sha256:00").is_err());
    assert!(SemanticCatalogDigest::parse("asc-catalog:v2:sha256:00").is_err());
}

#[test]
fn the_raw_digest_is_preserved_through_rebinding() {
    let identity = identity();
    assert!(identity.namespaces_are_distinct());
    let stored = raw('a');
    assert!(identity.preserves_raw(&stored));
    let rebound = identity.rebind(&stored).expect("identical raw rebinds");
    assert_eq!(rebound.raw, identity.raw);
    assert_eq!(
        identity.rebind(&raw('c')).expect_err("raw rewrite refused"),
        CatalogError::RawDigestRewritten
    );
}

#[test]
fn the_semantic_schema_label_is_validated() {
    assert_eq!(
        CatalogIdentity::new(raw('a'), semantic('b'), "").expect_err("empty schema"),
        CatalogError::InvalidSchema
    );
    let long = "s".repeat(200);
    assert_eq!(
        CatalogIdentity::new(raw('a'), semantic('b'), &long).expect_err("oversized schema"),
        CatalogError::InvalidSchema
    );
}

#[test]
fn raw_and_semantic_values_remain_distinct() {
    let identity = identity();
    assert_ne!(identity.raw.as_str(), identity.semantic.as_str());
    assert!(identity.raw.as_str().starts_with(RAW_CATALOG_PREFIX));
    assert!(
        identity
            .semantic
            .as_str()
            .starts_with(SEMANTIC_CATALOG_PREFIX)
    );
    assert!(!identity.raw.as_str().contains("asc-catalog"));
    assert!(!identity.semantic.as_str().starts_with("sha256:asc"));
}
