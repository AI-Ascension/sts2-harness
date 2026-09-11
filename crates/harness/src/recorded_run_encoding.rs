// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::sha256_hex;

pub(super) fn ndjson(rows: &[Value]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for row in rows {
        let bytes = canonical(row)?;
        if bytes.len() > 65_536 || out.len() + bytes.len() + 1 > 16 * 1024 * 1024 {
            return Err(String::from("output_record_or_stream_limit"));
        }
        out.extend(bytes);
        out.push(b'\n');
    }
    Ok(out)
}
pub(super) fn canonical(value: &Value) -> Result<Vec<u8>, String> {
    fn write(value: &Value, out: &mut String) -> Result<(), String> {
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
            Value::Number(v) => {
                let number = v.as_f64().ok_or_else(|| String::from("invalid_number"))?;
                out.push_str(ryu_js::Buffer::new().format_finite(number));
            }
            Value::String(v) => {
                out.push_str(&serde_json::to_string(v).map_err(|_| String::from("serialize"))?)
            }
            Value::Array(v) => {
                out.push('[');
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        out.push(',')
                    }
                    write(x, out)?
                }
                out.push(']')
            }
            Value::Object(v) => {
                out.push('{');
                let mut keys = v.keys().collect::<Vec<_>>();
                keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',')
                    }
                    out.push_str(&serde_json::to_string(k).map_err(|_| String::from("serialize"))?);
                    out.push(':');
                    write(v.get(*k).ok_or_else(|| String::from("missing"))?, out)?
                }
                out.push('}')
            }
        }
        Ok(())
    }
    let mut out = String::new();
    write(value, &mut out)?;
    Ok(out.into_bytes())
}

pub(super) struct Entry {
    path: String,
    media_type: &'static str,
    bytes: Vec<u8>,
}
impl Entry {
    pub(super) fn new(path: &str, media_type: &'static str, bytes: Vec<u8>) -> Self {
        Self {
            path: path.to_owned(),
            media_type,
            bytes,
        }
    }
    pub(super) fn manifest(&self) -> Value {
        json!({"path":self.path,"media_type":self.media_type,"bytes":self.bytes.len(),"sha256":sha256_hex(&self.bytes)})
    }
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            }
        }
    }
    !crc
}
fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend(value.to_le_bytes())
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_le_bytes())
}
pub(super) fn write_zip(output: &Path, entries: &[Entry]) -> Result<(), String> {
    if output.symlink_metadata().is_ok() {
        return Err(format!("refusing to overwrite {}", output.display()));
    }
    let parent = output
        .parent()
        .ok_or_else(|| String::from("output has no parent"))?;
    let temp = PathBuf::from(parent).join(format!(
        ".{}.tmp",
        output
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| String::from("output name invalid"))?
    ));
    let mut sorted = entries.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut bound = 22usize;
    for entry in &sorted {
        let limit = match entry.path.as_str() {
            "manifest.json" => 262_144,
            "reports/omissions.json" => 1_048_576,
            _ => 16 * 1024 * 1024,
        };
        bound += entry.bytes.len() + entry.path.len() * 2 + 76;
        if entry.bytes.len() > limit || bound > 16 * 1024 * 1024 {
            return Err(String::from("archive_or_entry_limit"));
        }
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for entry in sorted {
        let offset = u32::try_from(out.len()).map_err(|_| String::from("archive too large"))?;
        let name = entry.path.as_bytes();
        let size = u32::try_from(entry.bytes.len()).map_err(|_| String::from("entry too large"))?;
        let crc = crc32(&entry.bytes);
        put_u32(&mut out, 0x0403_4b50);
        put_u16(&mut out, 20);
        put_u16(&mut out, 0);
        put_u16(&mut out, 0);
        put_u16(&mut out, 0);
        put_u16(&mut out, 0);
        put_u32(&mut out, crc);
        put_u32(&mut out, size);
        put_u32(&mut out, size);
        put_u16(
            &mut out,
            u16::try_from(name.len()).map_err(|_| String::from("name too long"))?,
        );
        put_u16(&mut out, 0);
        out.extend(name);
        out.extend(&entry.bytes);
        put_u32(&mut central, 0x0201_4b50);
        put_u16(&mut central, 20);
        put_u16(&mut central, 20);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u32(&mut central, crc);
        put_u32(&mut central, size);
        put_u32(&mut central, size);
        put_u16(
            &mut central,
            u16::try_from(name.len()).map_err(|_| String::from("name too long"))?,
        );
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u32(&mut central, 0);
        put_u32(&mut central, offset);
        central.extend(name)
    }
    let central_offset = u32::try_from(out.len()).map_err(|_| String::from("archive too large"))?;
    out.extend(&central);
    put_u32(&mut out, 0x0605_4b50);
    put_u16(&mut out, 0);
    put_u16(&mut out, 0);
    put_u16(
        &mut out,
        u16::try_from(entries.len()).map_err(|_| String::from("too many entries"))?,
    );
    put_u16(
        &mut out,
        u16::try_from(entries.len()).map_err(|_| String::from("too many entries"))?,
    );
    put_u32(
        &mut out,
        u32::try_from(central.len()).map_err(|_| String::from("archive too large"))?,
    );
    put_u32(&mut out, central_offset);
    put_u16(&mut out, 0);
    if out.len() > 16 * 1024 * 1024 {
        return Err(String::from("archive_limit"));
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| format!("cannot create temp bundle: {e}"))?;
    file.write_all(&out)
        .map_err(|e| format!("cannot write bundle: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("cannot sync bundle: {e}"))?;
    // Link publication is atomic and cannot replace a concurrently created output.
    let published =
        fs::hard_link(&temp, output).map_err(|e| format!("cannot finalize bundle: {e}"));
    let _ = fs::remove_file(&temp);
    published
}
