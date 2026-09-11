// SPDX-License-Identifier: MIT

#[cfg(not(unix))]
fn main() {
    eprintln!("recorded-run export requires a Unix filesystem");
    std::process::exit(2);
}

#[cfg(unix)]
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = match args.as_slice() {
        [command, input, flag, output] if command == "export" && flag == "--output" => {
            sts2_harness::recorded_run::export_directory(
                std::path::Path::new(input),
                std::path::Path::new(output),
            )
            .map(print_report)
        }
        [command, input, flag, output] if command == "finalize" && flag == "--output" => {
            sts2_harness::recorded_run::finalize_after_controller(
                std::path::Path::new(input),
                std::path::Path::new(output),
            )
            .map(print_report)
        }
        _ => Err(String::from(
            "usage: sts2-recorded-run-export <export|finalize> <source-directory> --output <bundle.zip>",
        )),
    };
    if let Err(error) = result {
        eprintln!("recorded-run export failed: {error}");
        std::process::exit(2);
    }
}

#[cfg(unix)]
fn print_report(report: sts2_harness::recorded_run::ExportReport) {
    println!(
        "{{\"bundle_semantic_digest\":\"{}\",\"emitted_events\":{},\"emitted_accounting\":{}}}",
        report.bundle_semantic_digest, report.emitted_events, report.emitted_accounting
    );
}
