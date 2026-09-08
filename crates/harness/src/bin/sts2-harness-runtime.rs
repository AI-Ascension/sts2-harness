// SPDX-License-Identifier: MIT

#[path = "runtime_support/mod.rs"]
mod runtime_support;

fn main() {
    #[cfg(target_os = "linux")]
    {
        let mut arguments = std::env::args_os().skip(1);
        if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--worker-peer-verifier-v1")) {
            if arguments.next().is_some() {
                eprintln!("worker peer verifier rejects extra arguments");
                std::process::exit(2);
            }
            if sts2_harness::worker_local_linux::run_peer_verifier_from_stdin().is_err() {
                eprintln!("worker peer verifier failed");
                std::process::exit(2);
            }
            return;
        }
    }
    match runtime_support::RuntimeConfig::from_environment().and_then(runtime_support::run) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("sts2-harness runtime failed: {error}");
            std::process::exit(2);
        }
    }
}
