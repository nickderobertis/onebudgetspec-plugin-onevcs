//! Spawn the built artifact in an isolated working directory, also reused for install proof.
use std::{path::PathBuf, process::Command};

fn binary() -> PathBuf {
    std::env::var_os("SCAFFOLD_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/onebudgetspec-plugin-onevcs")
        })
}

#[test]
fn help_version_and_errors_perform_no_work() {
    let metadata = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(metadata.status.success());
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
    let package = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "onebudgetspec-plugin-onevcs")
        .unwrap();
    let expected = format!(
        "onebudgetspec-plugin-onevcs {}\n",
        package["version"].as_str().unwrap()
    );
    let cwd = tempfile::tempdir().unwrap();
    std::fs::write(cwd.path().join("sentinel"), "unchanged").unwrap();
    for args in [
        vec![],
        vec!["--help"],
        vec!["-h"],
        vec!["--version"],
        vec!["-V"],
        vec!["--report"],
        vec!["--help", "--report"],
        vec!["report"],
    ] {
        let output = assert_cmd::Command::new(binary())
            .current_dir(cwd.path())
            .args(&args)
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        match args.as_slice() {
            ["--version" | "-V"] => {
                assert!(output.status.success());
                assert_eq!(stdout, expected);
                assert!(stderr.is_empty());
            }
            [] | ["--help" | "-h"] => {
                assert!(output.status.success());
                assert!(stdout.contains("Usage: onebudgetspec-plugin-onevcs"));
                assert!(stdout.contains("scaffold"));
                assert!(stderr.is_empty());
            }
            _ => {
                assert_eq!(output.status.code(), Some(2));
                assert!(stdout.is_empty());
                assert!(stderr.contains("error: unsupported arguments"));
                assert!(stderr.contains("--help"));
            }
        }
        assert_eq!(
            std::fs::read_to_string(cwd.path().join("sentinel")).unwrap(),
            "unchanged"
        );
        assert_eq!(std::fs::read_dir(cwd.path()).unwrap().count(), 1);
    }
}
