// SPDX-License-Identifier: MIT

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let root =
            std::env::temp_dir().join(format!("controller-review-{}-{name}", std::process::id()));
        fs::create_dir_all(root.join("runs/run-1"))?;
        fs::create_dir_all(root.join("bundles"))?;
        let f = Self(root);
        f.script("exporter", "#!/bin/sh\n[ -f \"$2/result.json\" ] && [ -f \"$2/controller-exited\" ] || exit 9\nprintf exported > \"$4\"\n")?;
        Ok(f)
    }
    fn script(&self, name: &str, body: &str) -> Result<(), Box<dyn std::error::Error>> {
        fs::write(self.0.join(name), body)?;
        fs::set_permissions(self.0.join(name), fs::Permissions::from_mode(0o700))?;
        Ok(())
    }
    fn invoke(&self, script: &str) -> Result<std::process::Output, Box<dyn std::error::Error>> {
        Ok(
            Command::new(env!("CARGO_BIN_EXE_sts2-recorded-run-controller-finalize"))
                .arg(self.0.join("runs"))
                .arg(self.0.join("bundles"))
                .arg(self.0.join("exporter"))
                .args(["--", "/bin/sh", "-c", script, "fake"])
                .arg(self.0.join("runs/run-1"))
                .output()?,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn waits_after_stdout_closes_and_preserves_controller_failure()
-> Result<(), Box<dyn std::error::Error>> {
    for code in [0, 7] {
        let f = Fixture::new(&format!("wait-{code}"))?;
        let script = format!(
            "printf 'Controller running: %s\\n' \"$1\"; printf private; printf '\\n'; exec 1>&-; sleep 0.05; printf '{{}}' > \"$1/result.json\"; touch \"$1/controller-exited\"; exit {code}"
        );
        let output = f.invoke(&script)?;
        assert_eq!(output.status.code(), Some(code));
        assert!(output.stdout.is_empty());
        assert_eq!(
            fs::read(f.0.join("bundles/run-1.recorded-run.zip"))?,
            b"exported"
        );
    }
    Ok(())
}
#[test]
fn invalid_markers_missing_result_and_oversized_output_do_not_export()
-> Result<(), Box<dyn std::error::Error>> {
    for (name, script) in [
        ("missing", "printf 'Controller running: %s\\n' \"$1\""),
        (
            "duplicate",
            "printf 'Controller running: %s\\nController running: %s\\n' \"$1\" \"$1\"",
        ),
        ("outside", "printf 'Controller running: /tmp\\n'"),
        ("oversized", "head -c 1048577 /dev/zero"),
    ] {
        let f = Fixture::new(name)?;
        let output = f.invoke(script)?;
        assert_eq!(output.status.code(), Some(65));
        assert!(!f.0.join("bundles/run-1.recorded-run.zip").exists());
    }
    Ok(())
}
#[test]
fn export_failure_and_existing_output_preserve_inputs() -> Result<(), Box<dyn std::error::Error>> {
    let script = "printf 'Controller running: %s\\n' \"$1\"; printf '{}' > \"$1/result.json\"; touch \"$1/controller-exited\"";
    for existing in [true, false] {
        let f = Fixture::new(if existing { "existing" } else { "failure" })?;
        if existing {
            fs::write(f.0.join("bundles/run-1.recorded-run.zip"), b"old")?;
        } else {
            f.script("exporter", "#!/bin/sh\nexit 9\n")?;
        }
        assert_eq!(f.invoke(script)?.status.code(), Some(65));
        assert_eq!(fs::read(f.0.join("runs/run-1/result.json"))?, b"{}");
        if existing {
            assert_eq!(
                fs::read(f.0.join("bundles/run-1.recorded-run.zip"))?,
                b"old"
            );
        }
    }
    Ok(())
}
