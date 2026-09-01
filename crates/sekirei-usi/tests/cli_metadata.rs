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
        "Sekirei 0.3.21"
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
        "Sekirei 0.3.21"
    );
}
