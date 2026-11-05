//! Spawn the built artifact in an isolated working directory, also reused for install proof.
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

fn binary() -> PathBuf {
    std::env::var_os("PLUGIN_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/onebudgetspec-plugin-onevcs")
        })
}

/// The default invocation with nothing to reach: no credential, no `gh`, no network.
fn publish(stdin: &[u8]) -> std::process::Output {
    let cwd = tempfile::tempdir().unwrap();
    let mut child = Command::new(binary())
        .current_dir(cwd.path())
        .env_clear()
        .env("ONEVCS_GH", cwd.path().join("no-such-gh"))
        .env("ONEVCS_HOME", cwd.path().join("absent"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!cwd.path().join("absent").exists(), "no state is created");
    output
}

#[test]
fn help_version_and_usage_errors_perform_no_work() {
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
        vec!["--help"],
        vec!["-h"],
        vec!["history", "--help"],
        vec!["--version"],
        vec!["-V"],
        vec!["--report"],
        vec!["--help", "--report"],
        vec!["report"],
        vec!["history"],
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
            ["--help" | "-h", ..] => {
                assert!(output.status.success());
                assert!(stdout.contains("Usage: onebudgetspec-plugin-onevcs"));
                assert!(stdout.contains("PublishRequest"), "{stdout}");
                assert!(stdout.contains("history"), "{stdout}");
                assert!(stderr.is_empty());
            }
            ["history", "--help"] => {
                assert!(output.status.success());
                for flag in [
                    "--repo",
                    "--base-ref",
                    "--base-sha",
                    "--workflow",
                    "--artifact-name",
                    "--output",
                ] {
                    assert!(stdout.contains(flag), "{stdout}");
                }
                assert!(stdout.contains("RollingHistory"), "{stdout}");
            }
            _ => {
                assert_eq!(output.status.code(), Some(2));
                assert!(stdout.is_empty());
                assert!(stderr.contains("error:"), "{stderr}");
                assert!(stderr.contains("--help"), "{stderr}");
            }
        }
        assert_eq!(
            std::fs::read_to_string(cwd.path().join("sentinel")).unwrap(),
            "unchanged"
        );
        assert_eq!(std::fs::read_dir(cwd.path()).unwrap().count(), 1);
    }
}

#[test]
fn default_invocation_always_answers_one_delivery_result() {
    let head = "a".repeat(40);
    let request = serde_json::json!({
        "schema_version": 1, "destination": "https://github.com/owner/repo/pull/1",
        "key": "budget", "head_sha": head, "run_id": "1",
        "run_url": "https://github.com/owner/repo/actions/runs/1",
        "markdown": "## Budget report", "attempt": 1, "flags": [],
    });
    for (input, why) in [
        (b"".to_vec(), "malformed PublishRequest"),
        (b"{}".to_vec(), "malformed PublishRequest"),
        (
            request.to_string().into_bytes(),
            "could not read the standing comment",
        ),
    ] {
        let output = publish(&input);
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["delivered"], false, "{result}");
        assert!(result.get("url").is_none(), "{result}");
        assert!(
            result["diagnostic"].as_str().unwrap().contains(why),
            "{result}"
        );
    }
}

#[cfg(unix)]
#[test]
fn broken_output_streams_fail_without_panicking() {
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    for args in [
        &["--version"][..],
        &["--help"],
        &["--unknown"],
        &[],
        &[
            "history",
            "--repo",
            "owner/repo",
            "--base-ref",
            "main",
            "--base-sha",
            "0000000000000000000000000000000000000000",
            "--workflow",
            "1",
            "--artifact-name",
            "a",
            "--output",
            "/nonexistent-directory/rolling.json",
        ],
    ] {
        let (writer, reader) = UnixStream::pair().unwrap();
        drop(reader);
        let mut command = Command::new(binary());
        command.args(args).stdin(Stdio::null());
        let closed = Stdio::from(OwnedFd::from(writer));
        if args == ["--unknown"] {
            command.stderr(closed);
        } else {
            command.stdout(closed);
        }
        let result = command.output().unwrap();
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        assert!(!String::from_utf8_lossy(&result.stderr).contains("panicked"));
    }
}
