// SPDX-License-Identifier: MIT

use super::bundle::{MAP_MAX_PRESENTATION_HEIGHT, MAP_MAX_PRESENTATION_WIDTH, MapViewBundle};
use getrandom::fill as fill_random;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
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
            let digest = format!("{:x}", Sha256::digest(&png));
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
    (format!("{:x}", hasher.finalize()) == expected)
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
    let digest = format!("{:x}", Sha256::digest(&png));
    if manifest.contents.png_digest.as_deref() != Some(digest.as_str()) {
        return Err(());
    }
    let (width, height) = crate::episode::map::validate_png_bytes(
        &png,
        MAX_RENDER_BYTES,
        MAP_MAX_PRESENTATION_WIDTH,
        MAP_MAX_PRESENTATION_HEIGHT,
    )
    .map_err(|_| ())?;
    if u64::from(width)
        .checked_mul(u64::from(height))
        .is_none_or(|pixels| pixels > MAX_RENDER_PIXELS)
    {
        return Err(());
    }
    Ok((png, width, height))
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
