// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]

use std::fs;

use super::resources::HeldImage;
use super::test_support::Fixture;
use crate::TransportError;

#[test]
fn native_held_image_blocks_file_write_and_ancestor_rename() -> Result<(), TransportError> {
    let fixture = Fixture::new()?;
    let directory = fixture.directory.join("image-release");
    let renamed = fixture.directory.join("renamed-release");
    fs::create_dir(&directory).map_err(|_| TransportError::Os)?;
    let image_path = directory.join("synthetic-peer.exe");
    fs::copy(&fixture.image, &image_path).map_err(|_| TransportError::Os)?;
    let held = HeldImage::open(&image_path, &fixture.image_sha256)?;
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&image_path)
            .is_err()
    );
    assert!(fs::rename(&directory, &renamed).is_err());
    drop(held);
    // Positive controls show the failures were caused by the held locks,
    // not a permanently unwritable or unrenameable fixture.
    drop(
        fs::OpenOptions::new()
            .write(true)
            .open(&image_path)
            .map_err(|_| TransportError::Os)?,
    );
    fs::rename(&directory, &renamed).map_err(|_| TransportError::Os)?;
    Ok(())
}
