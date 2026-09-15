// SPDX-License-Identifier: MIT
//! Original isolated consumer of the harness-owned additive duplex frame contract.
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};
use std::fmt;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const VERSION: &str = "sts2.exo-lookup-wire-v1";
pub const FRAME_BYTES: usize = 196_608;
pub const TOOL_BYTES: usize = 16_384;
pub const FEEDBACK_BYTES: usize = 7_000;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub wire_version: String,
    pub request_id: String,
    pub turn_id: String,
    pub sequence: u64,
    pub payload: Payload,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Payload {
    Start {
        request: Value,
    },
    Query {
        arguments: Value,
    },
    ReadRetained {
        record_ordinal: usize,
        offset: usize,
    },
    Feedback {
        value: Value,
    },
    Decision {
        action_id: String,
    },
    Failure {
        code: String,
    },
}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

pub async fn read_line(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    (&mut *reader)
        .take((FRAME_BYTES + 2) as u64)
        .read_until(b'\n', &mut bytes)
        .await
        .map_err(|_| "exo_lookup_input")?;
    if bytes.last() != Some(&b'\n') || bytes.len() > FRAME_BYTES + 1 {
        return Err("exo_lookup_frame_bound");
    }
    bytes.pop();
    if bytes.is_empty() {
        return Err("exo_lookup_empty_frame");
    }
    Ok(bytes)
}

pub fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, &'static str> {
    if bytes.len() > FRAME_BYTES {
        return Err("exo_lookup_frame_bound");
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = Unique::deserialize(&mut decoder).map_err(|_| "exo_lookup_json")?;
    decoder.end().map_err(|_| "exo_lookup_json")?;
    serde_json::from_value(value.0).map_err(|_| "exo_lookup_shape")
}

pub fn feedback(
    bytes: &[u8],
    request: &str,
    turn: &str,
    sequence: u64,
) -> Result<Value, &'static str> {
    let frame: Frame = decode(bytes)?;
    if frame.wire_version != VERSION
        || frame.request_id != request
        || frame.turn_id != turn
        || frame.sequence != sequence
        || !(1..=32).contains(&sequence)
    {
        return Err("exo_lookup_identity");
    }
    let Payload::Feedback { value } = frame.payload else {
        return Err("exo_lookup_feedback_kind");
    };
    if serde_json::to_vec(&value)
        .map_err(|_| "exo_lookup_json")?
        .len()
        > FEEDBACK_BYTES
    {
        return Err("exo_lookup_feedback_bound");
    }
    Ok(value)
}

pub async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    frame: &Frame,
) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec(frame).map_err(|_| "exo_lookup_json")?;
    if bytes.len() > FRAME_BYTES {
        return Err("exo_lookup_frame_bound");
    }
    writer
        .write_all(&bytes)
        .await
        .map_err(|_| "exo_lookup_output")?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|_| "exo_lookup_output")?;
    writer.flush().await.map_err(|_| "exo_lookup_output")
}

struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = Value;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("unique JSON members")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
                Ok(v.into())
            }
            fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
                Ok(v.into())
            }
            fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
                Ok(v.into())
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Value, E> {
                Err(E::custom("integer required"))
            }
            fn visit_str<E>(self, v: &str) -> Result<Value, E> {
                Ok(v.into())
            }
            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = input.next_element::<Unique>()? {
                    values.push(value.0);
                }
                Ok(Value::Array(values))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = input.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("duplicate member"));
                    }
                    values.insert(key, input.next_value::<Unique>()?.0);
                }
                Ok(Value::Object(values))
            }
        }
        decoder.deserialize_any(JsonVisitor).map(Unique)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn feedback_is_closed_correlated_and_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let frame = Frame {
            wire_version: VERSION.into(),
            request_id: "request".into(),
            turn_id: "turn".into(),
            sequence: 1,
            payload: Payload::Feedback {
                value: json!({"data":"synthetic"}),
            },
        };
        let bytes = serde_json::to_vec(&frame)?;
        assert_eq!(
            feedback(&bytes, "request", "turn", 1)?,
            json!({"data":"synthetic"})
        );
        assert!(feedback(&bytes, "other", "turn", 1).is_err());
        assert!(feedback(&bytes, "request", "turn", 2).is_err());
        assert!(decode::<Value>(br#"{"a":{"x":1,"x":2}}"#).is_err());
        assert!(decode::<Value>(b"{}{}").is_err());
        let mut value = serde_json::to_value(frame)?;
        value["payload"]["value"] = json!("x".repeat(FEEDBACK_BYTES));
        assert!(feedback(&serde_json::to_vec(&value)?, "request", "turn", 1).is_err());
        Ok(())
    }
    #[tokio::test]
    async fn line_reader_preserves_following_frame_and_rejects_eof()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut input = &b"{}\n{\"a\":1}\n"[..];
        assert_eq!(read_line(&mut input).await?, b"{}");
        assert_eq!(read_line(&mut input).await?, br#"{"a":1}"#);
        assert!(read_line(&mut &b"{}"[..]).await.is_err());
        assert!(
            read_line(&mut &vec![b'x'; FRAME_BYTES + 2][..])
                .await
                .is_err()
        );
        Ok(())
    }
}
