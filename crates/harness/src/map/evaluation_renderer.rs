// SPDX-License-Identifier: MIT

use super::bundle::{MAP_MAX_PRESENTATION_HEIGHT, MAP_MAX_PRESENTATION_WIDTH, MapViewBundle};
use getrandom::fill as fill_random;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const RENDER_TIMEOUT: Duration = Duration::from_secs(5);
const RENDER_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_RENDERER_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RENDER_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_RENDER_BYTES: usize = 8 * 1024 * 1024;
const MAX_RENDER_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

pub(super) struct ImageCapture {
    pub(super) value: Option<Value>,
    pub(super) bytes: usize,
    pub(super) status: String,
}

impl ImageCapture {
    pub(super) fn unavailable(status: &'static str) -> Self {
        Self {
            value: None,
            bytes: 0,
            status: status.to_owned(),
        }
    }
}

pub(super) fn render_image(bundle: &MapViewBundle) -> ImageCapture {
    let Ok(binary) = std::env::var("STS2_MAP_RENDERER_BINARY") else {
        return ImageCapture::unavailable("unavailable_not_configured");
    };
    let Ok(expected_digest) = std::env::var("STS2_MAP_RENDERER_SHA256") else {
        return ImageCapture::unavailable("unavailable_digest_not_configured");
    };
    if !valid_digest(&expected_digest) || binary.is_empty() {
        return ImageCapture::unavailable("unavailable_renderer_config");
    }
    let path = PathBuf::from(binary);
    if verify_binary(&path, &expected_digest).is_err() {
        return ImageCapture::unavailable("unavailable_renderer_digest");
    }
    let Some(root) = temporary_root() else {
        return ImageCapture::unavailable("unavailable_renderer_temp");
    };
    let input = root.path.join("input");
    let output = root.path.join("output");
    if write_bundle_input(&input, bundle).is_err() {
        return ImageCapture::unavailable("unavailable_renderer_input");
    }
    if run_renderer(&path, &input, &output).is_err() {
        return ImageCapture::unavailable("unavailable_renderer_run");
    }
    let result = read_rendered_png(&output, bundle);
    match result {
        Ok((png, width, height)) => {
            let digest = crate::hex_bytes(Sha256::digest(&png));
            ImageCapture {
                value: Some(json!({
                    "media_type":"image/png",
                    "bytes_base64":base64(&png),
                    "graph_digest":bundle.manifest.snapshot_digest,
                    "sha256":digest,
                    "width":width,
                    "height":height
                })),
                bytes: png.len(),
                status: String::from("available"),
            }
        }
        Err(_) => ImageCapture::unavailable("unavailable_renderer_output"),
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn verify_binary(path: &Path, expected: &str) -> Result<(), ()> {
    let metadata = fs::metadata(path).map_err(|_| ())?;
    if !metadata.is_file() || metadata.len() > MAX_RENDERER_BYTES {
        return Err(());
    }
    let mut file = fs::File::open(path).map_err(|_| ())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(|_| ())?;
        if count == 0 {
            break;
        }
        total = total.checked_add(count as u64).ok_or(())?;
        if total > MAX_RENDERER_BYTES {
            return Err(());
        }
        hasher.update(&buffer[..count]);
    }
    (crate::hex_bytes(hasher.finalize()) == expected)
        .then_some(())
        .ok_or(())
}

struct TemporaryRoot {
    path: PathBuf,
}

impl Drop for TemporaryRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn temporary_root() -> Option<TemporaryRoot> {
    let temp = std::env::temp_dir();
    for _ in 0..8 {
        let mut random = [0_u8; 16];
        fill_random(&mut random).ok()?;
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = temp.join(format!(
            "sts2-map-evaluation-{}-{suffix}",
            std::process::id()
        ));
        match create_private_directory(&path) {
            Ok(()) => return Some(TemporaryRoot { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        builder.mode(0o700);
    }
    builder.create(path)
}

fn write_bundle_input(input: &Path, bundle: &MapViewBundle) -> Result<(), ()> {
    create_private_directory(input).map_err(|_| ())?;
    fs::write(input.join("visible-map.json"), &bundle.snapshot_bytes).map_err(|_| ())?;
    fs::write(
        input.join("analysis.json"),
        bundle.analysis_bytes().map_err(|_| ())?,
    )
    .map_err(|_| ())?;
    fs::write(
        input.join("manifest.json"),
        bundle.canonical_manifest_bytes().map_err(|_| ())?,
    )
    .map_err(|_| ())?;
    for (reference, bytes) in [
        (
            bundle.manifest.contents.decision_ref.as_str(),
            bundle.decision.as_deref(),
        ),
        (
            bundle.manifest.contents.viewer_ref.as_str(),
            bundle.viewer.as_deref(),
        ),
    ] {
        if !valid_file_reference(reference) {
            return Err(());
        }
        fs::write(input.join(reference), bytes.ok_or(())?).map_err(|_| ())?;
    }
    Ok(())
}

fn valid_file_reference(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    )
}

fn run_renderer(binary: &Path, input: &Path, output: &Path) -> Result<(), ()> {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .args(["render", "--bundle"])
        .arg(input)
        .arg("--out")
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for name in ["PATH", "SystemRoot", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(|_| ())?;
    let deadline = Instant::now() + RENDER_TIMEOUT;
    loop {
        match child.try_wait().map_err(|_| ())? {
            Some(status) if status.success() => return Ok(()),
            Some(_) => return Err(()),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(());
            }
            None => thread::sleep(RENDER_POLL_INTERVAL),
        }
    }
}

fn read_rendered_png(output: &Path, bundle: &MapViewBundle) -> Result<(Vec<u8>, u32, u32), ()> {
    let output_metadata = fs::symlink_metadata(output).map_err(|_| ())?;
    if output_metadata.file_type().is_symlink() || !output_metadata.is_dir() {
        return Err(());
    }
    let manifest_bytes =
        read_bounded_file(&output.join("manifest.json"), MAX_RENDER_MANIFEST_BYTES)?;
    let manifest = MapViewBundle::decode_manifest(&manifest_bytes).map_err(|_| ())?;
    if manifest.snapshot_digest != bundle.manifest.snapshot_digest {
        return Err(());
    }
    let png_reference = manifest.contents.png_ref.as_deref().ok_or(())?;
    if !valid_file_reference(png_reference) {
        return Err(());
    }
    let png = read_bounded_file(&output.join(png_reference), MAX_RENDER_BYTES)?;
    let digest = crate::hex_bytes(Sha256::digest(&png));
    if manifest.contents.png_digest.as_deref() != Some(digest.as_str()) {
        return Err(());
    }
    let (width, height) = validate_png_bytes(
        &png,
        MAX_RENDER_BYTES,
        MAP_MAX_PRESENTATION_WIDTH,
        MAP_MAX_PRESENTATION_HEIGHT,
    )?;
    if u64::from(width)
        .checked_mul(u64::from(height))
        .is_none_or(|pixels| pixels > MAX_RENDER_PIXELS)
    {
        return Err(());
    }
    Ok((png, width, height))
}

fn validate_png_bytes(
    bytes: &[u8],
    max_bytes: usize,
    max_width: u32,
    max_height: u32,
) -> Result<(u32, u32), ()> {
    if max_bytes == 0
        || max_width == 0
        || max_height == 0
        || bytes.len() > max_bytes
        || bytes.len() < PNG_SIGNATURE.len()
        || !bytes.starts_with(PNG_SIGNATURE)
        || !ends_at_iend(bytes)
    {
        return Err(());
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODED_BYTES,
    });
    let (width, height) = {
        let header = decoder.read_header_info().map_err(|_| ())?;
        (header.width, header.height)
    };
    if width == 0 || height == 0 || width > max_width || height > max_height {
        return Err(());
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(())?;
    if pixels > u64::from(max_width).checked_mul(u64::from(max_height)).ok_or(())? {
        return Err(());
    }
    let mut reader = decoder.read_info().map_err(|_| ())?;
    if reader.info().animation_control.is_some() {
        return Err(());
    }
    let decoded_bytes = reader.output_buffer_size().ok_or(())?;
    if decoded_bytes > MAX_DECODED_BYTES {
        return Err(());
    }
    let mut decoded = vec![0_u8; decoded_bytes];
    let output = reader.next_frame(&mut decoded).map_err(|_| ())?;
    if output.width != width || output.height != height {
        return Err(());
    }
    reader.finish().map_err(|_| ())?;
    Ok((width, height))
}

fn ends_at_iend(bytes: &[u8]) -> bool {
    let mut cursor = PNG_SIGNATURE.len();
    while let Some(header_end) = cursor.checked_add(8) {
        if header_end > bytes.len() {
            return false;
        }
        let length = u32::from_be_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]) as usize;
        let data_end = match header_end.checked_add(length) {
            Some(end) if end <= bytes.len() => end,
            _ => return false,
        };
        let chunk_end = match data_end.checked_add(4) {
            Some(end) if end <= bytes.len() => end,
            _ => return false,
        };
        if &bytes[cursor + 4..header_end] == b"IEND" {
            return length == 0 && chunk_end == bytes.len();
        }
        cursor = chunk_end;
    }
    false
}

fn read_bounded_file(path: &Path, max_bytes: usize) -> Result<Vec<u8>, ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes as u64
    {
        return Err(());
    }
    let mut file = fs::File::open(path).map_err(|_| ())?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    (bytes.len() <= max_bytes).then_some(bytes).ok_or(())
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[((first & 3) << 4 | second >> 4) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[((second & 15) << 2 | third >> 6) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}
