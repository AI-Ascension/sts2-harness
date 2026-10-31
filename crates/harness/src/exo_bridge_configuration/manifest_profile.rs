// SPDX-License-Identifier: MIT
//! Additive descriptor for the explicitly selected whole-content-manifest lookup profile.
use super::*;

impl Loaded {
    pub fn lookup_manifest_description(&self) -> Result<Value, &'static str> {
        let mut value = self.lookup_history_description()?;
        value["schema"] = json!("sts2.exo-lookup-capability-v4-content-manifest");
        value["wire_version"] = json!(crate::exo_lookup_wire::EXO_LOOKUP_MANIFEST_WIRE);
        value["profile"] = json!("content-manifest");
        value["tools"] = json!([
            "sts2_lookup_query",
            "sts2_lookup_read",
            "sts2_lookup_bootstrap",
            "sts2_lookup_history",
            "sts2_lookup_content_manifest",
            "sts2_lookup_manifest_read"
        ]);
        value["tool_digest"] = json!(sha256_hex(
            b"sts2_lookup_query\nsts2_lookup_read\nsts2_lookup_bootstrap\nsts2_lookup_history\nsts2_lookup_content_manifest\nsts2_lookup_manifest_read\n"
        ));
        value["content_manifest"] = json!({
            "protocol_version":crate::game_information::manifest::PROFILE,
            "schema_digest":crate::game_information::manifest::SCHEMA_DIGEST,
            "max_response_bytes":crate::game_information::manifest::MAX_RESPONSE_BYTES,
            "max_records":crate::game_information::manifest::MAX_RECORDS,
            "complete_retention_required":true
        });
        Ok(value)
    }
}
