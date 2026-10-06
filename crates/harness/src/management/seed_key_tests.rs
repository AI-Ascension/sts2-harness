// SPDX-License-Identifier: MIT

use std::fs;
use std::path::PathBuf;

use super::{
    FileSeedDerivationKeyAuthority, Keyring, SeedDerivationKeyAuthority, SeedKeyError,
    decode_lower_hex, derive_seed, framed, hmac_sha256,
};

#[test]
fn hmac_sha256_matches_the_published_known_answer() {
    let actual = hmac_sha256(b"my secret and secure key", b"input message")
        .map(|bytes| super::hex_lower(&bytes));
    assert_eq!(
        actual.ok().as_deref(),
        Some("97d2a569059bbcd8ead4444ff99071f4c01d005bcefe0d3567e1be628e5fdcd9")
    );
}

#[test]
fn length_prefixes_distinguish_different_field_partitions() {
    let left = framed(&[b"ab", b"c"]);
    let right = framed(&[b"a", b"bc"]);
    assert!(left.is_ok());
    assert!(right.is_ok());
    assert_ne!(left.ok(), right.ok());
}

#[test]
fn keyring_pins_identity_and_rejects_same_version_material_replacement() {
    let first_text = keyring_text("active", "11");
    let second_text = keyring_text("active", "22");
    let first = Keyring::parse(first_text.as_bytes());
    let second = Keyring::parse(second_text.as_bytes());
    assert!(first.is_ok());
    assert!(second.is_ok());
    let first = match first {
        Ok(value) => value,
        Err(_) => return,
    };
    let second = match second {
        Ok(value) => value,
        Err(_) => return,
    };
    let first_key = first.current_key();
    let second_key = second.current_key();
    assert!(first_key.is_ok());
    assert!(second_key.is_ok());
    let first_key = match first_key {
        Ok(value) => value,
        Err(_) => return,
    };
    let second_key = match second_key {
        Ok(value) => value,
        Err(_) => return,
    };
    assert_eq!(first_key.identity().authority_id, "service-a");
    assert_eq!(first_key.identity().version, "active");
    let transcript = framed(&[b"ascension.seed-derive-once/v2", b"test operation"]);
    assert!(transcript.is_ok());
    let transcript = match transcript {
        Ok(value) => value,
        Err(_) => return,
    };
    let first_seed = derive_seed(&first_key, &transcript);
    let replay_seed = derive_seed(&first_key, &transcript);
    assert!(first_seed.is_ok());
    assert_eq!(first_seed, replay_seed);
    assert_eq!(first_seed.as_ref().map(String::len).ok(), Some(32));
    assert!(
        !first_key
            .verifies_identity(second_key.identity())
            .unwrap_or(false)
    );
    assert!(
        first_key
            .verifies_identity(first_key.identity())
            .unwrap_or(false)
    );
}

#[test]
fn keyring_parser_rejects_unknown_duplicate_and_noncanonical_key_fields() {
    let valid = keyring_text("active", "11");
    let unknown = format!("{valid}unexpected=value\n");
    let duplicate = format!("{valid}current_version=active\n");
    let uppercase = keyring_text("active", "AA");
    let crlf = valid.replace('\n', "\r\n");
    assert!(Keyring::parse(unknown.as_bytes()).is_err());
    assert!(Keyring::parse(duplicate.as_bytes()).is_err());
    assert!(Keyring::parse(uppercase.as_bytes()).is_err());
    assert!(Keyring::parse(crlf.as_bytes()).is_err());
    assert!(decode_lower_hex::<32>(&"AA".repeat(32)).is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn private_linux_keyring_loads_by_descriptor_and_rejects_symlinks_and_open_modes() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let directory = TestDirectory::create();
    let keyring_path = directory.path().join("keys.conf");
    assert!(fs::write(&keyring_path, keyring_text("active", "11")).is_ok());
    assert!(fs::set_permissions(&keyring_path, fs::Permissions::from_mode(0o600)).is_ok());
    let authority = FileSeedDerivationKeyAuthority::open(&keyring_path);
    assert!(authority.is_ok());
    let authority = match authority {
        Ok(value) => value,
        Err(_) => return,
    };
    let current = authority.current_key();
    assert!(current.is_ok());
    let current = match current {
        Ok(value) => value,
        Err(_) => return,
    };
    assert_eq!(current.identity().version, "active");
    drop(current);
    drop(authority);

    let alias = directory.path().join("alias.conf");
    assert!(symlink(&keyring_path, &alias).is_ok());
    assert!(matches!(
        FileSeedDerivationKeyAuthority::open(&alias),
        Err(SeedKeyError::InsecureKeyring)
    ));

    assert!(fs::set_permissions(&keyring_path, fs::Permissions::from_mode(0o644)).is_ok());
    assert!(matches!(
        FileSeedDerivationKeyAuthority::open(&keyring_path),
        Err(SeedKeyError::InsecureKeyring)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn private_linux_keyring_rejects_symlinked_parent_and_nonregular_files() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let directory = TestDirectory::create();
    let target = directory.path().join("target");
    let alias = directory.path().join("alias");
    assert!(fs::write(&target, keyring_text("active", "11")).is_ok());
    assert!(fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).is_ok());
    assert!(symlink(directory.path(), &alias).is_ok());
    let through_alias = alias.join("target");
    assert!(matches!(
        FileSeedDerivationKeyAuthority::open(&through_alias),
        Err(SeedKeyError::InsecureKeyring)
    ));

    let directory_path = directory.path().join("not-a-file");
    assert!(fs::create_dir(&directory_path).is_ok());
    assert!(fs::set_permissions(&directory_path, fs::Permissions::from_mode(0o700)).is_ok());
    assert!(matches!(
        FileSeedDerivationKeyAuthority::open(&directory_path),
        Err(SeedKeyError::InsecureKeyring)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn private_linux_keyring_rejects_unsafe_ancestor_even_when_leaf_is_private() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::create();
    let unsafe_ancestor = directory.path().join("writable-parent");
    let private_parent = unsafe_ancestor.join("private");
    assert!(fs::create_dir(&unsafe_ancestor).is_ok());
    assert!(fs::set_permissions(&unsafe_ancestor, fs::Permissions::from_mode(0o777)).is_ok());
    assert!(fs::create_dir(&private_parent).is_ok());
    assert!(fs::set_permissions(&private_parent, fs::Permissions::from_mode(0o700)).is_ok());
    let keyring_path = private_parent.join("keys.conf");
    assert!(fs::write(&keyring_path, keyring_text("active", "11")).is_ok());
    assert!(fs::set_permissions(&keyring_path, fs::Permissions::from_mode(0o600)).is_ok());

    assert!(matches!(
        FileSeedDerivationKeyAuthority::open(&keyring_path),
        Err(SeedKeyError::InsecureKeyring)
    ));
}

#[cfg(not(target_os = "linux"))]
#[test]
fn protected_keyring_file_is_typed_unsupported_without_platform_controls() {
    assert!(matches!(
        FileSeedDerivationKeyAuthority::open(std::path::Path::new("/not-read")),
        Err(SeedKeyError::UnsupportedPlatform)
    ));
}

fn keyring_text(version: &str, repeated_hex: &str) -> String {
    format!(
        "schema=ascension.seed-keyring/v1\nauthority_id=service-a\ncurrent_version={version}\nkey.{version}={}\n",
        repeated_hex.repeat(32)
    )
}

#[cfg(target_os = "linux")]
struct TestDirectory(PathBuf);

#[cfg(target_os = "linux")]
impl TestDirectory {
    fn create() -> Self {
        use std::os::unix::fs::PermissionsExt;

        let path =
            std::env::temp_dir().join(format!("sts2-seed-keyring-test-{}", uuid::Uuid::new_v4()));
        assert!(fs::create_dir(&path).is_ok());
        assert!(fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).is_ok());
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(target_os = "linux")]
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
