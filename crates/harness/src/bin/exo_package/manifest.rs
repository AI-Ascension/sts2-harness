// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::Path;

use super::{Result, filesystem};
use sts2_harness::{EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoToolCatalog, sha256_hex};

pub(super) const MAX_MANIFEST: usize = 32 * 1024;
const MIB: u64 = 1024 * 1024;
const ROOT: &str = "sts2.exo-install-manifest-v1";

pub(super) struct Input {
    pub name: &'static str,
    pub maximum: u64,
    pub executable: bool,
    pub owned: Option<&'static [u8]>,
}

macro_rules! owned {
    ($name:literal, $source:literal) => {
        Input {
            name: $name,
            maximum: 8 * MIB,
            executable: false,
            owned: Some(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../",
                $source
            ))),
        }
    };
}

// Closed, sorted, flat inventory: an extra module or nested payload is never packaged.
pub(super) const INPUTS: &[Input] = &[
    Input {
        name: "LICENSE.exo",
        maximum: MIB,
        executable: false,
        owned: None,
    },
    owned!("LICENSE.harness", "LICENSE"),
    Input {
        name: "LICENSE.node",
        maximum: 8 * MIB,
        executable: false,
        owned: None,
    },
    owned!("THIRD_PARTY_NOTICES.harness", "THIRD_PARTY_NOTICES.md"),
    owned!(
        "executor.Cargo.lock",
        "experiments/exo-agent/bridge/Cargo.lock"
    ),
    Input {
        name: "exo.Cargo.lock",
        maximum: 8 * MIB,
        executable: false,
        owned: None,
    },
    Input {
        name: "exo.pnpm-lock.yaml",
        maximum: 8 * MIB,
        executable: false,
        owned: None,
    },
    owned!(
        "extension.package.json",
        "experiments/exo-agent/extension/package.json"
    ),
    owned!("harness.Cargo.lock", "Cargo.lock"),
    owned!("index.ts", "experiments/exo-agent/extension/src/index.ts"),
    Input {
        name: "node",
        maximum: 256 * MIB,
        executable: true,
        owned: None,
    },
    owned!("rust-toolchain.toml", "rust-toolchain.toml"),
    Input {
        name: "sts2-exo-bridge",
        maximum: 512 * MIB,
        executable: true,
        owned: None,
    },
    Input {
        name: "sts2-exo-executor",
        maximum: 512 * MIB,
        executable: true,
        owned: None,
    },
    Input {
        name: "sts2-exo-package",
        maximum: 512 * MIB,
        executable: true,
        owned: None,
    },
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Component {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub schema: String,
    pub platform: String,
    pub harness_revision: String,
    pub build_provenance: String,
    pub exo_revision: String,
    pub rust: String,
    pub node: String,
    pub pnpm: String,
    pub contract: String,
    pub tool_digest: String,
    pub components: Vec<Component>,
}

pub(super) fn build(input: &Path, revision: &str) -> Result<Manifest> {
    if !hex(revision, 40) {
        return Err("exo_package_source_revision");
    }
    let directory = filesystem::open_directory(input, false)?;
    let components = inspect(&directory)?;
    Ok(Manifest {
        schema: ROOT.to_owned(),
        platform: "linux-x86_64".to_owned(),
        harness_revision: revision.to_owned(),
        build_provenance: "operator_review_required".to_owned(),
        exo_revision: EXO_SOURCE_REVISION.to_owned(),
        rust: "1.97.1".to_owned(),
        node: "22.14.0".to_owned(),
        pnpm: "10.26.2".to_owned(),
        contract: EXO_CONTRACT_VERSION.to_owned(),
        tool_digest: ExoToolCatalog::reviewed().catalog_digest(),
        components,
    })
}

pub(super) fn inspect(directory: &File) -> Result<Vec<Component>> {
    let expected = INPUTS
        .iter()
        .map(|input| input.name.to_owned())
        .collect::<Vec<_>>();
    if filesystem::names(directory, INPUTS.len())? != expected {
        return Err("exo_package_inventory");
    }
    INPUTS
        .iter()
        .map(|input| {
            let file = filesystem::open_file(directory, input.name, false)?;
            filesystem::check_executable(&file, input.executable)?;
            let (bytes, digest) = filesystem::hash(file, input.maximum)?;
            if bytes == 0 || input.owned.is_some_and(|owned| sha256_hex(owned) != digest) {
                return Err("exo_package_owned_identity");
            }
            Ok(Component {
                name: input.name.to_owned(),
                bytes,
                sha256: digest,
            })
        })
        .collect()
}

pub(super) fn decode(bytes: &[u8]) -> Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| "exo_package_manifest")?;
    if manifest.schema != ROOT
        || manifest.platform != "linux-x86_64"
        || !hex(&manifest.harness_revision, 40)
        || manifest.build_provenance != "operator_review_required"
        || manifest.exo_revision != EXO_SOURCE_REVISION
        || manifest.rust != "1.97.1"
        || manifest.node != "22.14.0"
        || manifest.pnpm != "10.26.2"
        || manifest.contract != EXO_CONTRACT_VERSION
        || manifest.tool_digest != ExoToolCatalog::reviewed().catalog_digest()
        || manifest.components.len() != INPUTS.len()
        || encode(&manifest)? != bytes
    {
        return Err("exo_package_manifest");
    }
    for (component, input) in manifest.components.iter().zip(INPUTS) {
        if component.name != input.name
            || component.bytes == 0
            || component.bytes > input.maximum
            || !hex(&component.sha256, 64)
            || input
                .owned
                .is_some_and(|bytes| sha256_hex(bytes) != component.sha256)
        {
            return Err("exo_package_manifest_component");
        }
    }
    Ok(manifest)
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|_| "exo_package_encoding")?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn identity(manifest: &[u8], settings: &[u8]) -> String {
    sha256_hex(format!(
        "sts2.exo-install-v1\n{}\n{}\n",
        sha256_hex(manifest),
        sha256_hex(settings)
    ))
}
