//! Lightweight CLI metadata contract tests.

use std::io::Write;
use std::process::{Command, Stdio};

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

#[test]
fn usi_handshake_reports_the_same_package_version_as_version_flag() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sekirei"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to start sekirei");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"usi\nisready\nquit\n")
        .unwrap();
    let output = child
        .wait_with_output()
        .expect("failed to wait for sekirei");
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.lines().any(|line| line == "id name Sekirei"));
    assert!(
        stdout
            .lines()
            .any(|line| line == concat!("id version ", env!("CARGO_PKG_VERSION")))
    );
    assert!(stdout.lines().any(|line| line == "usiok"));
    assert!(stdout.lines().any(|line| line == "readyok"));
}
