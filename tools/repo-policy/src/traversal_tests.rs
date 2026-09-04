// SPDX-License-Identifier: MIT

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{collect, language_findings, relative_text, size_findings};
use crate::config::Policy;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "harness-policy-traversal-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root)?;
        Ok(Self(root))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn scans_rust_binary_sources_but_not_generated_bin_output() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = &fixture.0;
    for directory in ["crates/demo/src/bin", "managed/bin", "bin"] {
        fs::create_dir_all(root.join(directory))?;
    }
    fs::write(
        root.join("crates/demo/src/bin/demo.rs"),
        "// source\n".repeat(401),
    )?;
    fs::write(root.join("crates/demo/src/bin/forbidden.py"), "pass\n")?;
    fs::write(root.join("managed/bin/generated.py"), "pass\n")?;
    fs::write(root.join("bin/generated.rs"), "// generated\n".repeat(401))?;
    let policy =
        Policy::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml"))?;
    let files = collect(root, &policy)?;
    let paths: Vec<_> = files.iter().map(|path| relative_text(root, path)).collect();
    assert_eq!(
        paths,
        [
            "crates/demo/src/bin/demo.rs",
            "crates/demo/src/bin/forbidden.py"
        ]
    );
    let (_, size) = size_findings(root, &files, &policy);
    assert!(
        size.iter()
            .any(|finding| finding.rule == "SIZE001"
                && finding.path == "crates/demo/src/bin/demo.rs")
    );
    let language = language_findings(root, &files);
    assert_eq!(language.len(), 1);
    assert_eq!(language[0].path, "crates/demo/src/bin/forbidden.py");
    Ok(())
}
