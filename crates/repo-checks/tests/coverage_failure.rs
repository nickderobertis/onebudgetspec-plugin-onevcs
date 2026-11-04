//! Missing measurement tools must fail the real runner, never become an empty eval.
#![cfg(unix)]
use std::{fs, path::Path, process::Command};
#[test]
fn missing_coverage_plugin_reports_the_cause_and_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(env!("CARGO"), bin.join("cargo")).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/coverage.sh");
    let output = Command::new("/bin/bash")
        .arg(script)
        .arg("repo-checks")
        .current_dir(temp.path())
        .env("PATH", bin)
        .env("CARGO_HOME", temp.path().join("cargo-home"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("no such command"), "{error}");
    assert!(error.contains("could not read instrumentation environment"));
    assert!(error.contains("retry"));
}
