//! The real plugin binary and real onevcs against a local HTTP server answering as GitHub
//! answers the Actions token. Only that HTTP boundary (tests/support/actions_host.py) is
//! doubled.
#![cfg(unix)]

use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use serde_json::{Value, json};

const HEAD_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct ActionsHost {
    root: tempfile::TempDir,
    server: Child,
    url: String,
}

impl ActionsHost {
    fn new(head: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let state = json!({"head": head, "comments": []});
        std::fs::write(root.path().join("state.json"), state.to_string()).unwrap();
        let mut server = Command::new("python3")
            .arg(Self::program())
            .arg("serve")
            .arg(root.path().join("state.json"))
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut port = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        assert!(!port.is_empty(), "the HTTP server exited before listening");
        let url = format!("http://127.0.0.1:{}", port.trim());
        Self { root, server, url }
    }

    fn program() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/actions_host.py")
    }

    fn state(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.root.path().join("state.json")).unwrap())
            .unwrap()
    }

    fn update(&self, change: impl FnOnce(&mut Value)) {
        let mut state = self.state();
        change(&mut state);
        std::fs::write(self.root.path().join("state.json"), state.to_string()).unwrap();
    }

    /// One head's request through the plugin's default invocation, as the core CLI pipes it.
    fn report(&self, head: &str, markdown: &str) -> Value {
        self.update(|state| state["head"] = json!(head));
        let mut command = Command::new(env!("CARGO_BIN_EXE_onebudgetspec-plugin-onevcs"));
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", self.root.path())
            .env("GH_TOKEN", "actions-token")
            .env("ONEVCS_GH", Self::program())
            .env("ACTIONS_HOST", &self.url)
            .current_dir(self.root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut plugin = command.spawn().unwrap();
        let request = json!({
            "schema_version": 1,
            "destination": "https://github.com/owner/repo/pull/1",
            "key": "budget-report",
            "head_sha": head,
            "run_id": "1",
            "run_url": "https://github.com/owner/repo/actions/runs/1",
            "markdown": markdown,
            "attempt": 1,
            "flags": [],
        });
        plugin
            .stdin
            .take()
            .unwrap()
            .write_all(request.to_string().as_bytes())
            .unwrap();
        let output = plugin.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for ActionsHost {
    fn drop(&mut self) {
        self.server.kill().unwrap();
        self.server.wait().unwrap();
    }
}

#[test]
fn a_new_head_under_the_actions_token_updates_the_one_standing_comment() {
    let host = ActionsHost::new(HEAD_A);

    let first = host.report(HEAD_A, "first report");
    assert_eq!(first["delivered"], true, "{first}");
    let second = host.report(HEAD_B, "second report");
    assert_eq!(second["delivered"], true, "{second}");

    let state = host.state();
    let comments = state["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 1, "{comments:#?}");
    let body = comments[0]["body"].as_str().unwrap();
    assert!(body.contains("second report"), "{body}");
    assert!(!body.contains("first report"), "{body}");
    assert_eq!(first["url"], second["url"]);
}

#[test]
fn a_person_sharing_the_bots_login_keeps_their_comment() {
    let host = ActionsHost::new(HEAD_A);
    assert_eq!(host.report(HEAD_A, "first report")["delivered"], true);
    host.update(|state| state["comments"][0]["author"]["__typename"] = json!("User"));
    let their_comment = host.state()["comments"][0].clone();

    let reported = host.report(HEAD_B, "second report");

    assert_eq!(reported["delivered"], true, "{reported}");
    let state = host.state();
    let comments = state["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2, "{comments:#?}");
    assert_eq!(comments[0], their_comment);
    assert!(
        comments[1]["body"]
            .as_str()
            .unwrap()
            .contains("second report")
    );
}
