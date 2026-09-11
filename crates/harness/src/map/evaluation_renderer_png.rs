// SPDX-License-Identifier: MIT

use std::io::Cursor;

const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

pub(super) fn validate_png_bytes(
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
    let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or(())?;
    if pixels
        > u64::from(max_width)
            .checked_mul(u64::from(max_height))
            .ok_or(())?
    {
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
