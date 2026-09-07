// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::{Digest, Sha256};

pub(super) fn digest_value(domain: &str, value: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(canonical_json(value));
    format!("{:x}", hasher.finalize())
}

fn canonical_json(value: &Value) -> Vec<u8> {
    match value {
        Value::Null => b"null".to_vec(),
        Value::Bool(value) => value.to_string().into_bytes(),
        Value::Number(value) => value.to_string().into_bytes(),
        Value::String(value) => serde_json::to_vec(value).unwrap_or_default(),
        Value::Array(values) => {
            let mut bytes = vec![b'['];
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                bytes.extend(canonical_json(value));
            }
            bytes.push(b']');
            bytes
        }
        Value::Object(values) => {
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let mut bytes = vec![b'{'];
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                bytes.extend(serde_json::to_vec(key).unwrap_or_default());
                bytes.push(b':');
                bytes.extend(canonical_json(value));
            }
            bytes.push(b'}');
            bytes
        }
    }
}
