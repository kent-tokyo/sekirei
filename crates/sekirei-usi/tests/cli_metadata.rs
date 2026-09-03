//! Lightweight CLI metadata contract tests.

use std::process::Command;

#[test]
fn version_flag_reports_package_version_without_starting_usi() {
    let output = Command::new(env!("CARGO_BIN_EXE_sekirei"))
        .arg("--version")
        .output()
        .expect("failed to run sekirei --version");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("Sekirei ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn short_version_flag_is_supported() {
    let output = Command::new(env!("CARGO_BIN_EXE_sekirei"))
        .arg("-V")
        .output()
        .expect("failed to run sekirei -V");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("Sekirei ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_flag_describes_usi_usage_without_starting_the_loop() {
    let output = Command::new(env!("CARGO_BIN_EXE_sekirei"))
        .arg("--help")
        .output()
        .expect("failed to run sekirei --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("USI shogi engine"));
    assert!(stdout.contains("sekirei [NNUE_WEIGHTS]"));
    assert!(stdout.contains("The engine reads USI commands from stdin."));
}
