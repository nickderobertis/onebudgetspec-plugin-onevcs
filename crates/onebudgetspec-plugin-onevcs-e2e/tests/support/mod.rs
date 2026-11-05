//! Real installed executables and real git; only GitHub's HTTP boundary is doubled.
//!
//! Each journey binary compiles this module and uses its own part of it.
#![allow(dead_code)]
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

use onebudgetspec_core::comparison::{BudgetRun, ComparisonInput, RollingHistory};
use serde_json::{Value, json};

pub const REPOSITORY: &str = "owner/repo";
pub const BASE_REF: &str = "main";
pub const WORKFLOW: u64 = 42;
pub const ARTIFACT: &str = "budget-results";
pub const CHANGE: &str = "https://github.com/owner/repo/pull/1";
pub const KEY: &str = "budget-report";
pub const ROLLING_FILE: &str = "rolling-history.json";

pub fn plugin() -> PathBuf {
    std::env::var_os("PLUGIN_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace().join("target/debug/onebudgetspec-plugin-onevcs"))
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The released core CLI: `just dependencies` installs the version package.json pins.
pub fn core_cli() -> PathBuf {
    std::env::var_os("ONEBUDGETSPEC_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace().join("node_modules/.bin/onebudgetspec"))
}

/// One `onebudgetspec schema` root, from the installed consumer's view of the contract.
pub fn schema_root(root: &str) -> Value {
    let output = Command::new(core_cli())
        .args(["schema", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let bundle: Value = serde_json::from_slice(&output.stdout).unwrap();
    bundle["roots"][root].clone()
}

pub fn assert_conforms(root: &str, instance: &Value) {
    let schema = schema_root(root);
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<_> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{root}: {errors:?}");
}

/// The external host. `ONEVCS_HOME` is empty for its whole life, which `drop` holds it to.
pub struct Host {
    pub root: tempfile::TempDir,
    server: Child,
    url: String,
    home: PathBuf,
}

impl Host {
    pub fn new(head: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("archives")).unwrap();
        let home = root.path().join("onevcs-home");
        std::fs::create_dir(&home).unwrap();
        let host_state = json!({"head": head, "comments": [], "runs": [], "artifacts": {}});
        std::fs::write(root.path().join("state.json"), host_state.to_string()).unwrap();
        let mut server = Command::new("python3")
            .arg(Self::program())
            .arg("serve")
            .arg(root.path())
            .stderr(std::fs::File::create(root.path().join("server.stderr")).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let port = loop {
            let record = std::fs::read_to_string(root.path().join("port")).unwrap_or_default();
            if let Some(port) = record.strip_suffix('\n') {
                break port.to_owned();
            }
            let exited = server.try_wait().unwrap();
            assert!(
                exited.is_none() && Instant::now() < deadline,
                "HTTP boundary did not start ({exited:?}): {}",
                std::fs::read_to_string(root.path().join("server.stderr")).unwrap()
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        Self {
            server,
            url: format!("http://127.0.0.1:{port}"),
            home,
            root,
        }
    }

    fn program() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/boundary.py")
    }

    /// No ambient auth, Actions variables or registry: only what a runner must supply.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", self.root.path())
            .env("ONEVCS_HOME", &self.home)
            .env("GH_TOKEN", "boundary-token")
            .env("ONEVCS_GH", Self::program())
            .env("BOUNDARY_HOST", &self.url)
            .current_dir(self.root.path());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        command
    }

    pub fn state(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.root.path().join("state.json")).unwrap())
            .unwrap()
    }

    pub fn update(&self, change: impl FnOnce(&mut Value)) {
        let mut state = self.state();
        change(&mut state);
        std::fs::write(self.root.path().join("state.json"), state.to_string()).unwrap();
    }

    pub fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.path().join("requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    pub fn clear_requests(&self) {
        let _ = std::fs::remove_file(self.root.path().join("requests.jsonl"));
    }

    pub fn writes(&self) -> usize {
        self.requests()
            .iter()
            .filter(|r| r["method"] == "POST" && r["path"] != "/graphql" || r["method"] == "PATCH")
            .count()
    }

    pub fn paths(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|r| r["path"].as_str().unwrap().to_owned())
            .collect()
    }

    /// The body as GitHub holds it, marker included, for the comment on that index.
    pub fn comment_body(&self, index: usize) -> String {
        self.state()["comments"][index]["body"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Record a completed run as the host lists it, ordered by the given creation minute.
    pub fn add_run(&self, id: u64, head: &str, attempt: u32, minute: u32) {
        self.add_listed_run(json!({
            "id": id, "head_sha": head, "run_attempt": attempt,
            "created_at": format!("2026-11-01T{:02}:{:02}:00Z", minute / 60, minute % 60),
        }));
    }

    pub fn add_listed_run(&self, facts: Value) {
        let mut run = json!({
            "workflow_id": WORKFLOW, "name": "budgets", "event": "push", "head_branch": BASE_REF,
            "status": "completed", "conclusion": "success", "run_attempt": 1,
            "created_at": "2026-11-01T00:00:00Z", "run_started_at": null,
            "updated_at": "2026-11-01T23:59:00Z",
        });
        for (key, value) in facts.as_object().unwrap() {
            run[key] = value.clone();
        }
        self.update(|state| state["runs"].as_array_mut().unwrap().push(run));
    }

    /// Set a listed run's latest attempt, as a rerun does.
    pub fn rerun(&self, id: u64, attempt: u32) {
        self.update(|state| {
            let runs = state["runs"].as_array_mut().unwrap();
            let run = runs.iter_mut().find(|r| r["id"] == id).unwrap();
            run["run_attempt"] = json!(attempt);
        });
    }

    /// One completed upload: an immutable archive attached to its run.
    pub fn upload(&self, run_id: u64, artifact_id: u64, name: &str, bytes: &[u8]) {
        let path = self
            .root
            .path()
            .join("archives")
            .join(format!("{artifact_id}.zip"));
        std::fs::write(path, bytes).unwrap();
        self.attach(run_id, artifact_id, name, bytes.len() as u64, false);
    }

    pub fn attach(&self, run_id: u64, artifact_id: u64, name: &str, size: u64, expired: bool) {
        self.update(|state| {
            let artifacts = &mut state["artifacts"];
            let listed = artifacts[run_id.to_string()].take();
            let mut listed = listed.as_array().cloned().unwrap_or_default();
            listed.push(json!({
                "id": artifact_id, "name": name, "size_in_bytes": size, "expired": expired,
            }));
            artifacts[run_id.to_string()] = json!(listed);
        });
    }

    /// Drive the installed `history` verb inside `checkout`.
    pub fn history(&self, checkout: &Path, base_sha: &str) -> HistoryAnswer {
        let output = self.root.path().join("rolling.out.json");
        self.run_history(checkout, base_sha, &output)
    }

    pub fn run_history(&self, checkout: &Path, base_sha: &str, path: &Path) -> HistoryAnswer {
        let run = self
            .command(plugin())
            .current_dir(checkout)
            .args(["history", "--repo", REPOSITORY, "--base-ref", BASE_REF])
            .args(["--base-sha", base_sha])
            .args(["--workflow", &WORKFLOW.to_string()])
            .args(["--artifact-name", ARTIFACT])
            .arg("--output")
            .arg(path)
            .output()
            .unwrap();
        let report = serde_json::from_slice(&run.stdout).unwrap_or_else(|error| {
            panic!(
                "{error}: {}{}",
                String::from_utf8_lossy(&run.stdout),
                String::from_utf8_lossy(&run.stderr)
            )
        });
        HistoryAnswer {
            code: run.status.code(),
            report,
            stderr: String::from_utf8_lossy(&run.stderr).into_owned(),
            history: std::fs::read(path)
                .ok()
                .map(|bytes| serde_json::from_slice(&bytes).unwrap()),
        }
    }

    /// Drive real core over the real plugin as its reporter.
    pub fn report(&self, input: &ComparisonInput) -> Reported {
        let path = self.root.path().join("input.json");
        std::fs::write(&path, serde_json::to_vec(input).unwrap()).unwrap();
        let output = self
            .command(core_cli())
            .args(["report", "--json", "--input"])
            .arg(&path)
            .arg("--reporter")
            .arg(plugin())
            .args(["--destination", CHANGE, "--key", KEY])
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!("{error}: {output:?}");
        });
        Reported {
            code: output.status.code(),
            report,
        }
    }

    /// Pipe one raw request to the plugin's default invocation.
    pub fn publish_raw(&self, request: &str) -> Output {
        let mut child = self
            .command(plugin())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(request.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.server.kill().unwrap();
        self.server.wait().unwrap();
        if !std::thread::panicking() {
            let held: Vec<_> = std::fs::read_dir(&self.home).unwrap().collect();
            assert!(held.is_empty(), "ONEVCS_HOME gained state: {held:?}");
        }
    }
}

pub struct HistoryAnswer {
    pub code: Option<i32>,
    pub report: Value,
    pub stderr: String,
    pub history: Option<Value>,
}

impl HistoryAnswer {
    pub fn parsed(&self) -> RollingHistory {
        serde_json::from_value(self.history.clone().expect(&self.stderr)).unwrap()
    }

    pub fn heads(&self) -> Vec<String> {
        self.parsed().runs.into_iter().map(|r| r.head_sha).collect()
    }

    pub fn disclosed(&self, list: &str) -> Vec<String> {
        self.report[list]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["head_sha"].as_str().unwrap().to_owned())
            .collect()
    }

    pub fn reason_for(&self, list: &str, head: &str) -> String {
        self.report[list]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["head_sha"] == head)
            .unwrap_or_else(|| panic!("{head} not in {list}: {}", self.report))["reason"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

pub struct Reported {
    pub code: Option<i32>,
    pub report: Value,
}

impl Reported {
    pub fn diagnostics(&self) -> String {
        self.report["diagnostics"].to_string()
    }

    pub fn delivered(&self) -> bool {
        !self.diagnostics().contains("report not delivered")
    }
}

/// A real repository, with commits on `main` unless the journey checks out elsewhere.
pub struct Repository {
    pub directory: tempfile::TempDir,
    made: std::cell::Cell<u32>,
}

impl Repository {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let repository = Self {
            directory,
            made: std::cell::Cell::new(0),
        };
        repository.git(&["init", "--quiet", "--initial-branch", BASE_REF]);
        repository
    }

    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    pub fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(self.path())
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.test",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    pub fn commit(&self) -> String {
        self.made.set(self.made.get() + 1);
        let message = format!("commit {}", self.made.get());
        self.git(&["commit", "--quiet", "--allow-empty", "-m", &message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// A commit on its own branch, leaving the current branch where it was.
    pub fn divergent_commit(&self, from: &str) -> String {
        let back = self.git(&["rev-parse", "HEAD"]);
        self.git(&["checkout", "--quiet", "--detach", from]);
        let commit = self.commit();
        self.git(&["branch", &format!("fork-{commit}")]);
        self.git(&["checkout", "--quiet", "--detach", &back]);
        commit
    }
}

/// A completed ordinary run: `rows` budgets, each measured by `value(row)`.
pub fn budget_run(
    head: &str,
    run_id: u64,
    attempt: u32,
    rows: usize,
    value: impl Fn(usize) -> f64,
) -> BudgetRun {
    let threshold = 100.0;
    let (catalog, results): (Vec<_>, Vec<_>) = (0..rows)
        .map(|row| {
            let id = format!("budget-{row:03}");
            let actual = value(row);
            let direction = onebudgetspec_core::Direction::Max;
            let (verdict, headroom, percent) =
                onebudgetspec_core::judge(direction, threshold, actual);
            (
                json!({
                    "id": id, "file": "budgets.yaml", "description": null, "labels": ["report"],
                    "measure": "reported", "command": ["node", "measure.js"], "unit": "ms",
                    "direction": "max", "threshold": threshold, "timeout_seconds": null,
                }),
                json!({
                    "id": id, "file": "budgets.yaml", "labels": ["report"], "unit": "ms",
                    "direction": "max", "threshold": threshold, "verdict": verdict,
                    "actual": actual, "headroom": headroom, "headroom_percent": percent,
                    "detail": null, "error": null,
                    "started_at": "2026-11-01T00:00:00Z", "ended_at": "2026-11-01T00:00:01Z",
                    "host": {"load1": null, "cpus": 1, "mem_available_mib": null, "conditions": {}},
                }),
            )
        })
        .unzip();
    serde_json::from_value(json!({
        "schema_version": 1, "repository": REPOSITORY, "head_sha": head, "base_ref": BASE_REF,
        "run_id": run_id.to_string(),
        "run_url": format!("https://github.com/owner/repo/actions/runs/{run_id}"),
        "attempt": attempt, "completed_at": "2026-11-01T00:00:02Z",
        "catalog": catalog,
        "results": {"schema_version": 1, "results": results},
        "unavailable": [],
    }))
    .unwrap()
}

pub fn rolling(runs: Vec<BudgetRun>) -> RollingHistory {
    RollingHistory {
        schema_version: 1,
        repository: REPOSITORY.into(),
        base_ref: BASE_REF.into(),
        runs,
    }
}

pub fn archive(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut bytes);
    for (name, content) in members {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap();
    bytes.into_inner()
}

pub fn rolling_archive(history: &RollingHistory) -> Vec<u8> {
    let json = serde_json::to_vec(history).unwrap();
    archive(&[(ROLLING_FILE, &json), ("budget-run.json", b"{}")])
}

/// What the action's base job does around the history verb, which owns no carry-forward:
/// read one history, drop this commit's earlier attempt, append, trim to 20, upload.
pub fn carried_forward(previous: Option<RollingHistory>, run: BudgetRun) -> RollingHistory {
    let mut history = previous.unwrap_or_else(|| rolling(Vec::new()));
    history.runs.retain(|known| known.head_sha != run.head_sha);
    history.runs.push(run);
    let excess = history.runs.len().saturating_sub(20);
    history.runs.drain(..excess);
    history
}
