//! The real plugin binary, real onevcs and real `gh` against a local HTTP server answering
//! with what `tests/fixtures/github-wire.json` recorded from GitHub. Only that HTTP boundary
//! (tests/support/actions_host.py) is doubled.
#![cfg(unix)]

use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use serde_json::{Value, json};

const HEAD_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const RERECORD: &str = "the recording no longer matches; run `just record-github-wire` \
                        and review what moved";

fn support(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(file)
}

fn wire() -> Value {
    serde_json::from_slice(&std::fs::read(support("tests/fixtures/github-wire.json")).unwrap())
        .unwrap()
}

struct ActionsHost {
    root: tempfile::TempDir,
    server: Child,
    proxy: String,
}

impl ActionsHost {
    fn new(head: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let state = json!({"head": head, "comments": [], "queries": []});
        std::fs::write(root.path().join("state.json"), state.to_string()).unwrap();
        let mut server = Command::new("python3")
            .arg(support("tests/support/actions_host.py"))
            .arg("serve")
            .arg(root.path().join("state.json"))
            .arg(support("tests/fixtures/github-wire.json"))
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut port = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        assert!(!port.is_empty(), "the HTTP server exited before listening");
        let proxy = format!("http://127.0.0.1:{}", port.trim());
        Self {
            root,
            server,
            proxy,
        }
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
    /// `gh` treats `github.localhost` as plain HTTP, and the proxy setting sends that here.
    fn report(&self, head: &str, markdown: &str) -> Value {
        self.update(|state| state["head"] = json!(head));
        let mut command = Command::new(env!("CARGO_BIN_EXE_onebudgetspec-plugin-onevcs"));
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", self.root.path())
            .env("GH_CONFIG_DIR", self.root.path().join("gh"))
            .env("GH_HOST", "github.localhost")
            .env("GH_TOKEN", "actions-token")
            .env("GH_ENTERPRISE_TOKEN", "actions-token")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("HTTP_PROXY", &self.proxy)
            .current_dir(self.root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut plugin = command.spawn().unwrap();
        let request = json!({
            "schema_version": 1,
            "destination": wire()["source"]["actions_change"],
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
    host.update(|state| {
        state["comments"][0]["author"]["__typename"] =
            wire()["person_comment"]["author"]["__typename"].clone();
    });
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

/// The drift gate: a `onevcs` upgrade that asks GitHub anything new fails here, offline,
/// until the fixture both HTTP doubles answer from is recorded again.
#[test]
fn onevcs_sends_exactly_the_recorded_query() {
    let host = ActionsHost::new(HEAD_A);
    assert_eq!(host.report(HEAD_A, "report")["delivered"], true);

    let state = host.state();
    let queries = state["queries"].as_array().unwrap();
    assert!(!queries.is_empty(), "the plugin never read the comments");
    for query in queries {
        assert_eq!(query, &wire()["query"], "{RERECORD}");
    }
}

/// The identities the Actions regression depends on, as recorded.
#[test]
fn the_recording_holds_the_actions_bots_two_spellings() {
    let wire = wire();
    assert_eq!(wire["actions_comment"]["author"]["__typename"], "Bot");
    assert_ne!(
        wire["actions_comment"]["author"]["login"], wire["actions_rest_comment"]["user"]["login"],
        "{RERECORD}"
    );
}

mod recording {
    use super::*;

    const CHANGE: &str = "https://github.com/owner/repo/pull/1";

    fn rest_answer() -> Value {
        json!({"id": 7, "node_id": "IC_1", "body": "secret body", "extra": "dropped",
               "url": "https://api.github.com/repos/owner/repo/issues/comments/7",
               "html_url": format!("{CHANGE}#issuecomment-7"),
               "issue_url": "https://api.github.com/repos/owner/repo/issues/1",
               "user": {"login": "github-actions[bot]", "type": "Bot", "extra": "dropped"}})
    }

    fn page(author_types: &[&str], next_cursor: Option<&str>) -> Value {
        let nodes: Vec<Value> = author_types
            .iter()
            .map(|author_type| {
                json!({"id": "IC_1", "databaseId": 7, "url": format!("{CHANGE}#issuecomment-7"),
                       "body": "secret body",
                       "author": {"__typename": author_type, "login": "github-actions"}})
            })
            .collect();
        json!({"data": {"viewer": {"login": "someone"}, "repository": {"pullRequest": {
            "headRefOid": "f".repeat(40),
            "comments": {
                "nodes": nodes,
                "pageInfo": {"hasNextPage": next_cursor.is_some(), "endCursor": next_cursor}}}}}})
    }

    /// A `gh` on PATH answering one first comment page and, for a cursor, a second.
    fn stub_gh_pages(directory: &Path, first: &Value, second: &Value) {
        let rest = rest_answer();
        std::fs::write(directory.join("first.json"), first.to_string()).unwrap();
        std::fs::write(directory.join("second.json"), second.to_string()).unwrap();
        std::fs::write(directory.join("rest.json"), rest.to_string()).unwrap();
        write_gh(
            directory,
            &format!(
                "echo \"$*\" >> {0}/calls\n\
                 case \"$*\" in\n\
                 *after=*) cat {0}/second.json ;;\n\
                 *graphql*) cat {0}/first.json ;;\n\
                 *) cat {0}/rest.json ;;\n\
                 esac\n",
                directory.display()
            ),
        );
    }

    fn stub_gh(directory: &Path, author_types: &[&str]) {
        stub_gh_pages(directory, &page(author_types, None), &page(&[], None));
    }

    fn write_gh(directory: &Path, body: &str) {
        let gh = directory.join("gh");
        std::fs::write(&gh, format!("#!/bin/sh\n{body}")).unwrap();
        Command::new("chmod").arg("+x").arg(&gh).status().unwrap();
    }

    fn recorder(directory: &Path) -> Command {
        let mut command = Command::new("python3");
        let path = format!("{}:{}", directory.display(), std::env::var("PATH").unwrap());
        command
            .env_clear()
            .env("PATH", path)
            .env("HOME", directory)
            .arg(support("tests/support/record_github_wire.py"));
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        command
    }

    fn record(directory: &Path, output: &Path) -> std::process::Output {
        record_with(directory, real_plugin(), output)
    }

    fn record_with(directory: &Path, plugin: &Path, output: &Path) -> std::process::Output {
        recorder(directory)
            .arg("--plugin")
            .arg(plugin)
            .arg("--output")
            .arg(output)
            .args(["--actions-change", CHANGE, "--person-change", CHANGE])
            .output()
            .unwrap()
    }

    /// Asserts a refused recording named `reason` and left the existing fixture alone.
    fn assert_refused_keeping_fixture(directory: &Path, plugin: &Path, reason: &str) {
        let output = directory.join("wire.json");
        std::fs::write(&output, "kept").unwrap();
        let refused = record_with(directory, plugin, &output);
        assert!(!refused.status.success(), "{refused:?}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(stderr.contains(reason), "{stderr}");
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "kept");
    }

    fn real_plugin() -> &'static Path {
        Path::new(env!("CARGO_BIN_EXE_onebudgetspec-plugin-onevcs"))
    }

    /// A plugin that answers `answer` without reading anything.
    fn stub_plugin(directory: &Path, answer: &str) -> PathBuf {
        let plugin = directory.join("stub-plugin");
        std::fs::write(
            &plugin,
            format!("#!/bin/sh\ncat > /dev/null\necho '{answer}'\n"),
        )
        .unwrap();
        Command::new("chmod")
            .arg("+x")
            .arg(&plugin)
            .status()
            .unwrap();
        plugin
    }

    #[test]
    fn records_what_the_plugin_read_with_bodies_elided_and_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        stub_gh(directory.path(), &["Bot", "User"]);
        let output = directory.path().join("wire.json");

        let recorded = record(directory.path(), &output);

        assert!(recorded.status.success(), "{recorded:?}");
        let wire: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert!(wire["query"].as_str().unwrap().contains("viewer"), "{wire}");
        assert_eq!(wire["actions_comment"]["author"]["__typename"], "Bot");
        assert_eq!(wire["person_comment"]["author"]["__typename"], "User");
        assert_eq!(
            wire["actions_rest_comment"],
            json!({"id": 7, "node_id": "IC_1", "body": "",
                   "url": "https://api.github.com/repos/owner/repo/issues/comments/7",
                   "html_url": format!("{CHANGE}#issuecomment-7"),
                   "issue_url": "https://api.github.com/repos/owner/repo/issues/1",
                   "user": {"login": "github-actions[bot]", "type": "Bot"}})
        );
        assert!(!wire.to_string().contains("secret body"), "{wire}");
        let calls = std::fs::read_to_string(directory.path().join("calls")).unwrap();
        assert!(!calls.contains("--method"), "a recording wrote: {calls}");
    }

    #[test]
    fn fails_without_a_fixture_when_a_change_lacks_the_author_type() {
        let directory = tempfile::tempdir().unwrap();
        stub_gh(directory.path(), &["Bot"]);
        let output = directory.path().join("wire.json");

        let refused = record(directory.path(), &output);

        assert!(!refused.status.success(), "{refused:?}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(stderr.contains("holds no comment by"), "{stderr}");
        assert!(!output.exists(), "a failed recording must leave no fixture");
    }

    #[test]
    fn finds_an_author_on_a_later_page_and_records_a_single_page() {
        let directory = tempfile::tempdir().unwrap();
        stub_gh_pages(
            directory.path(),
            &page(&["Bot"], Some("cursor")),
            &page(&["User"], None),
        );
        let output = directory.path().join("wire.json");

        let recorded = record(directory.path(), &output);

        assert!(recorded.status.success(), "{recorded:?}");
        let wire: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(wire["person_comment"]["author"]["__typename"], "User");
        assert_eq!(
            wire["page_info"],
            json!({"hasNextPage": false, "endCursor": null})
        );
    }

    #[test]
    fn a_failing_gh_keeps_the_existing_fixture_and_names_the_change() {
        let directory = tempfile::tempdir().unwrap();
        write_gh(
            directory.path(),
            "echo 'HTTP 401: Bad credentials' >&2\nexit 1\n",
        );
        let output = directory.path().join("wire.json");
        std::fs::write(&output, "kept").unwrap();

        let refused = record(directory.path(), &output);

        assert!(!refused.status.success(), "{refused:?}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(
            stderr.contains(&format!("could not read {CHANGE}")),
            "{stderr}"
        );
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "kept");
    }

    #[test]
    fn another_bots_comment_is_not_recorded_as_the_actions_bots() {
        let directory = tempfile::tempdir().unwrap();
        let mut dependabot = page(&["Bot", "User"], None);
        dependabot["data"]["repository"]["pullRequest"]["comments"]["nodes"][0]["author"]["login"] =
            json!("dependabot");
        stub_gh_pages(directory.path(), &dependabot, &page(&[], None));
        assert_refused_keeping_fixture(directory.path(), real_plugin(), "holds no comment by");
    }

    #[test]
    fn a_failing_rest_read_keeps_the_existing_fixture() {
        let directory = tempfile::tempdir().unwrap();
        stub_gh(directory.path(), &["Bot", "User"]);
        let graphql = directory.path().join("first.json");
        write_gh(
            directory.path(),
            &format!(
                "case \"$*\" in\n*graphql*) cat {} ;;\n*) exit 1 ;;\nesac\n",
                graphql.display()
            ),
        );
        assert_refused_keeping_fixture(directory.path(), real_plugin(), "over REST");
    }

    #[test]
    fn a_rest_answer_about_another_comment_keeps_the_existing_fixture() {
        for change in ["id", "user", "html_url", "url", "issue_url"] {
            let directory = tempfile::tempdir().unwrap();
            stub_gh(directory.path(), &["Bot", "User"]);
            let mut rest = rest_answer();
            match change {
                "id" => rest["id"] = json!(8),
                "user" => rest["user"]["login"] = json!("dependabot[bot]"),
                "url" => rest["url"] = json!("https://api.github.com/repos/o/r/issues/comments/7"),
                "issue_url" => rest["issue_url"] = json!(null),
                _ => drop(rest.as_object_mut().unwrap().remove(change)),
            }
            std::fs::write(directory.path().join("rest.json"), rest.to_string()).unwrap();
            assert_refused_keeping_fixture(
                directory.path(),
                real_plugin(),
                "is not that Actions comment",
            );
        }
    }

    #[test]
    fn only_the_refused_head_after_real_reads_counts_as_a_finished_recording() {
        for answer in [
            r#"{"delivered":true,"url":"u"}"#,
            r#"{"delivered":false,"diagnostic":"something else"}"#,
            r#"{"delivered":false,"diagnostic":"expected head x but nothing was read"}"#,
            r#"{"delivered":false,"diagnostic":1}"#,
            "[]",
            "not json",
        ] {
            let directory = tempfile::tempdir().unwrap();
            stub_gh(directory.path(), &["Bot", "User"]);
            let plugin = stub_plugin(directory.path(), answer);
            assert_refused_keeping_fixture(directory.path(), &plugin, "could not read");
        }
    }

    #[test]
    fn an_unusable_comment_page_keeps_the_existing_fixture() {
        for answer in [
            json!({"errors": [{"message": "refused"}]}),
            json!({"data": {"repository": {"pullRequest": null}}}),
            json!({"data": {"repository": {"pullRequest": {"comments": {
                "nodes": [{"id": "IC_1"}], "pageInfo": {}}}}}}),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let read = json!({"query": "query", "answer": answer});
            let plugin = directory.path().join("stub-plugin");
            std::fs::write(
                &plugin,
                format!(
                    "#!/bin/sh\ncat > /dev/null\necho '{read}' > \"$RECORD_GITHUB_WIRE_READS\"\n\
                     echo '{{\"delivered\":false,\"diagnostic\":\"expected head x\"}}'\n"
                ),
            )
            .unwrap();
            Command::new("chmod")
                .arg("+x")
                .arg(&plugin)
                .status()
                .unwrap();
            assert_refused_keeping_fixture(directory.path(), &plugin, "unusable comment page");
        }
    }

    #[test]
    fn refuses_a_change_that_is_not_a_github_pull_request() {
        let directory = tempfile::tempdir().unwrap();
        stub_gh(directory.path(), &["Bot", "User"]);
        let refused = recorder(directory.path())
            .args([
                "--plugin",
                env!("CARGO_BIN_EXE_onebudgetspec-plugin-onevcs"),
            ])
            .args(["--output", "unused.json"])
            .args(["--actions-change", "https://example.com/owner/repo/pull/1"])
            .args(["--person-change", CHANGE])
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(!refused.status.success(), "{refused:?}");
        assert!(String::from_utf8_lossy(&refused.stderr).contains("expected https://github.com"));
        assert!(!directory.path().join("calls").exists());
    }

    #[test]
    fn refuses_anything_but_a_graphql_read_while_standing_in_for_gh() {
        let directory = tempfile::tempdir().unwrap();
        stub_gh(directory.path(), &["Bot"]);
        let read = "query=query { viewer { login } }";
        let mutation = "query=mutation { addComment { clientMutationId } }";
        let smuggled = ["api", "graphql", "-f", read, "-f", mutation];
        let writes: [&[&str]; 6] = [
            &smuggled,
            &[
                "api",
                "--method",
                "PATCH",
                "repos/owner/repo/issues/comments/7",
            ],
            &["api", "graphql", "-f", mutation],
            &["api", "graphql", "--method", "DELETE", "-f", read],
            &["api", "graphql", "-X", "DELETE", "-f", read],
            &["api", "graphql", "--method=DELETE", "-f", read],
        ];
        for write in writes {
            let refused = recorder(directory.path())
                .env("RECORD_GITHUB_WIRE_READS", directory.path().join("reads"))
                .args(write)
                .output()
                .unwrap();
            assert!(!refused.status.success(), "{write:?}: {refused:?}");
            assert!(String::from_utf8_lossy(&refused.stderr).contains("read-only"));
        }
        assert!(!directory.path().join("calls").exists());
    }
}
