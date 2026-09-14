// SPDX-License-Identifier: MIT

use super::PrivateRoot;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Parent(PathBuf);

impl Parent {
    fn new() -> Result<Self> {
        let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .canonicalize()?;
        let path = target.join(format!("exo-private-root-test-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        // Only this test's exclusively created parent is owned by this guard.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn relative_temporary_parent_is_rejected() {
    assert!(PrivateRoot::create_under(Path::new("relative"), uuid::Uuid::new_v4()).is_err());
}

#[test]
fn symlink_temporary_parent_is_rejected() -> Result {
    let parent = Parent::new()?;
    let real = parent.0.join("real");
    std::fs::create_dir(&real)?;
    let link = parent.0.join("link");
    symlink(&real, &link)?;
    assert!(PrivateRoot::create_under(&link, uuid::Uuid::new_v4()).is_err());
    assert_eq!(std::fs::read_dir(real)?.count(), 0);
    Ok(())
}

#[test]
fn existing_private_child_is_not_reused_or_removed() -> Result {
    let parent = Parent::new()?;
    let identity = uuid::Uuid::new_v4();
    let existing = parent.0.join(format!("sts2-exo-{identity}"));
    std::fs::create_dir(&existing)?;
    let marker = existing.join("marker");
    std::fs::write(&marker, b"existing child")?;
    assert!(PrivateRoot::create_under(&parent.0, identity).is_err());
    let linked_identity = uuid::Uuid::new_v4();
    let link = parent.0.join(format!("sts2-exo-{linked_identity}"));
    symlink(&existing, &link)?;
    assert!(PrivateRoot::create_under(&parent.0, linked_identity).is_err());
    assert!(std::fs::symlink_metadata(link)?.file_type().is_symlink());
    assert_eq!(std::fs::read(marker)?, b"existing child");
    Ok(())
}

#[test]
fn cleanup_is_confined_to_the_owned_private_child() -> Result {
    let parent = Parent::new()?;
    let sibling = parent.0.join("sibling");
    std::fs::write(&sibling, b"preserved")?;
    let private = PrivateRoot::create_under(&parent.0, uuid::Uuid::new_v4())?;
    for path in [
        private.0.clone(),
        private.0.join("state"),
        private.0.join("temp"),
    ] {
        assert_eq!(std::fs::metadata(path)?.permissions().mode() & 0o777, 0o700);
    }
    private.remove()?;
    assert!(!private.0.exists());
    assert_eq!(std::fs::read(sibling)?, b"preserved");
    Ok(())
}
