//! Real core reporter protocol, real plugin and real onevcs against the HTTP boundary.
#![cfg(unix)]
mod support;

use onebudgetspec_core::comparison::{BudgetRun, ComparisonInput};
use serde_json::{Value, json};
use support::{CHANGE, Host, KEY, assert_conforms, budget_run};

const HEAD_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HEAD_C: &str = "cccccccccccccccccccccccccccccccccccccccc";

/// Five saved base commits measuring about 10 to 14.
fn baseline(rows: usize) -> Vec<BudgetRun> {
    (0..5u8)
        .map(|n| {
            let head = format!("{:040x}", u32::from(n) + 1);
            budget_run(&head, 500 + u64::from(n), 1, rows, |_| 10.0 + f64::from(n))
        })
        .collect()
}

fn input(head: &str, run: u64, attempt: u32, value: f64) -> ComparisonInput {
    ComparisonInput {
        schema_version: 1,
        current: budget_run(head, run, attempt, 1, |_| value),
        baseline: baseline(1),
    }
}

const ADVERSE: f64 = 50.0;
const IMPROVED: f64 = 5.0;
const OVER: f64 = 200.0;

const EARLIER: &str = "<details><summary>Flags on earlier heads (";

/// The collapsed section core writes for earlier heads' flags, if the body has one.
fn earlier_section(body: &str) -> Option<&str> {
    let (_, section) = body.split_once(EARLIER)?;
    Some(section.split_once("</details>").unwrap().0)
}

/// One head's lines inside the collapsed earlier-heads section; empty when not listed.
fn flags_of(body: &str, head: &str) -> String {
    earlier_section(body)
        .unwrap_or_default()
        .lines()
        .skip_while(|line| !line.starts_with(head))
        .take_while(|line| line.starts_with(head) || line.starts_with("- "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The hidden rolling history core embeds in a standing comment.
fn history_of(body: &str) -> Value {
    let embedded = body
        .split_once("<!-- onebudgetspec-history\n")
        .unwrap()
        .1
        .split_once("\n-->")
        .unwrap()
        .0;
    serde_json::from_str(embedded).unwrap()
}

#[test]
fn create_update_identical_retry_keep_one_comment_and_visible_adverse_history() {
    let host = Host::new(HEAD_A);
    host.update(|state| state["description"] = json!("unrelated description"));

    let first = host.report(&input(HEAD_A, 1, 1, ADVERSE));
    assert_eq!(first.code, Some(0), "{}", first.report);
    assert!(first.delivered(), "{}", first.diagnostics());
    let created = host.comment_body(0);
    assert!(created.contains("## Budget report"), "{created}");
    assert!(created.contains("| budgets.yaml / budget-000 (ms) | within | 50.000"));
    // The current head's flag is the report's own row; only earlier heads are collapsed.
    assert_eq!(earlier_section(&created), None, "{created}");
    assert!(!created.contains("adverse flag(s)"), "{created}");
    let history = history_of(&created);
    assert_eq!(history["entries"][0]["head_sha"], HEAD_A);
    assert_eq!(history["entries"][0]["flags"][0]["id"], "budget-000");
    assert_eq!(host.writes(), 1);
    assert_eq!(
        host.paths().last().unwrap(),
        "/repos/owner/repo/issues/1/comments"
    );

    // One read before the merge, and onevcs's own head check before the write.
    assert_eq!(host.paths().len(), 3);
    host.clear_requests();
    let retry = host.report(&input(HEAD_A, 1, 1, ADVERSE));
    assert_eq!(retry.code, Some(0));
    assert!(retry.delivered(), "{}", retry.diagnostics());
    assert_eq!(host.comment_body(0), created);
    assert_eq!(host.writes(), 0, "an identical retry writes nothing");

    // A new head that improved: the first head's flag moves into the collapsed section,
    // and the new head, flagged nowhere, gains no line.
    host.update(|state| state["head"] = json!(HEAD_B));
    let improved = host.report(&input(HEAD_B, 2, 1, IMPROVED));
    assert_eq!(improved.code, Some(0));
    assert!(improved.delivered(), "{}", improved.diagnostics());
    let after_improvement = host.comment_body(0);
    assert!(
        after_improvement.contains("<details><summary>Flags on earlier heads (1)</summary>"),
        "{after_improvement}"
    );
    let first_flags = flags_of(&after_improvement, HEAD_A);
    assert_eq!(
        first_flags,
        format!("{HEAD_A} (attempt 1): 1 adverse flag(s)\n- budgets.yaml/budget-000: 24.03 SD"),
        "{after_improvement}"
    );
    assert_eq!(flags_of(&after_improvement, HEAD_B), "");
    assert!(!after_improvement.contains("0 adverse flag(s)"));
    assert!(
        after_improvement.find(EARLIER) < after_improvement.find("<!-- onebudgetspec-history"),
        "the section sits above the hidden history: {after_improvement}"
    );

    // A failing head: the earlier section is unchanged and the caller's failure stands.
    host.update(|state| state["head"] = json!(HEAD_C));
    let failing = host.report(&input(HEAD_C, 3, 1, OVER));
    assert_eq!(failing.code, Some(1), "{}", failing.report);
    assert!(failing.delivered(), "{}", failing.diagnostics());
    let after_failure = host.comment_body(0);
    assert!(after_failure.contains("| budgets.yaml / budget-000 (ms) | over | 200.000"));
    assert!(after_failure.contains("Flags on earlier heads (1)"));
    assert_eq!(flags_of(&after_failure, HEAD_A), first_flags);
    assert_eq!(flags_of(&after_failure, HEAD_B), "");
    assert_eq!(
        flags_of(&after_failure, HEAD_C),
        "",
        "the current head is not earlier"
    );
    assert_eq!(
        history_of(&after_failure)["entries"][2]["flags"][0]["id"],
        "budget-000"
    );

    // A rerun of that head replaces its entry; the earlier attempt cannot regress it.
    let rerun = host.report(&input(HEAD_C, 3, 2, IMPROVED));
    assert_eq!(rerun.code, Some(0));
    let after_rerun = host.comment_body(0);
    let rerun_entry = &history_of(&after_rerun)["entries"][2];
    assert_eq!(rerun_entry["head_sha"], HEAD_C);
    assert_eq!(rerun_entry["attempt"], 2);
    assert_eq!(rerun_entry["flags"], json!([]));
    assert_eq!(flags_of(&after_rerun, HEAD_A), first_flags);
    host.clear_requests();
    let late = host.report(&input(HEAD_C, 3, 1, OVER));
    assert_eq!(
        late.code,
        Some(1),
        "the old attempt's verdict is still its own"
    );
    assert_eq!(host.comment_body(0), after_rerun);
    assert_eq!(host.writes(), 0);

    let state = host.state();
    assert_eq!(state["comments"].as_array().unwrap().len(), 1);
    assert_eq!(state["description"], "unrelated description");
}

#[test]
fn heads_that_carry_no_flags_post_no_earlier_heads_section() {
    let host = Host::new(HEAD_A);
    for (head, run) in [(HEAD_A, 1), (HEAD_B, 2), (HEAD_C, 3)] {
        host.update(|state| state["head"] = json!(head));
        let reported = host.report(&input(head, run, 1, IMPROVED));
        assert_eq!(reported.code, Some(0), "{}", reported.report);
        assert!(reported.delivered(), "{}", reported.diagnostics());
        let body = host.comment_body(0);
        assert!(!body.contains("Flags on earlier heads"), "{body}");
        assert!(!body.contains("<details>"), "{body}");
        assert!(!body.contains("adverse flag(s)"), "{body}");
        assert_eq!(
            history_of(&body)["entries"].as_array().unwrap().len(),
            run as usize
        );
    }
    assert_eq!(host.state()["comments"].as_array().unwrap().len(), 1);
}

/// What the v0.1.1 plugin (core 0.1.6) posted for a flagged head, then an unflagged one,
/// captured from that release's binary against this host double.
const V0_1_1_BODY: &str = concat!(
    "## Budget report\n\nhead: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n\n",
    "### Per-head adverse flags\n\n",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa (attempt 1): 1 adverse flag(s)\n",
    "- budgets.yaml/latency: 2.50 SD\n\n",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb (attempt 1): 0 adverse flag(s)\n",
    "<!-- onebudgetspec-history\n",
    r#"{"schema_version":1,"entries":[{"head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","run_id":"1","run_url":"https://github.com/owner/repo/actions/runs/1","attempt":1,"flags":[{"file":"budgets.yaml","id":"latency","sigma_change":2.5,"constant_baseline_change":false}]},{"head_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","run_id":"2","run_url":"https://github.com/owner/repo/actions/runs/2","attempt":1,"flags":[]}],"omitted_heads":0}"#,
    "\n-->\n\n",
    "<!-- onevcs:standing-comment:v1:d8992162ab96e90c64661d4880f7a0b0d10f8085a51dd0c200d2438ed7f46601 -->",
);

#[test]
fn a_comment_the_previous_release_posted_is_updated_in_place_with_its_flags_carried() {
    let host = Host::new(HEAD_C);
    let previous = json!({
        "id": 100, "node_id": "owned-100", "author": "viewer", "body": V0_1_1_BODY,
        "html_url": "https://github.com/owner/repo/pull/1#issuecomment-100",
    });
    host.update(|state| state["comments"] = json!([previous]));

    let reported = host.report(&input(HEAD_C, 3, 1, IMPROVED));
    assert_eq!(reported.code, Some(0), "{}", reported.report);
    assert!(reported.delivered(), "{}", reported.diagnostics());
    let state = host.state();
    let comments = state["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 1, "no second comment: {comments:?}");
    assert_eq!(comments[0]["id"], 100);
    assert_eq!(host.writes(), 1);

    let body = host.comment_body(0);
    assert!(!body.contains("### Per-head adverse flags"), "{body}");
    assert!(body.contains("Flags on earlier heads (1)"), "{body}");
    assert_eq!(
        flags_of(&body, HEAD_A),
        format!("{HEAD_A} (attempt 1): 1 adverse flag(s)\n- budgets.yaml/latency: 2.50 SD")
    );
    assert_eq!(flags_of(&body, HEAD_B), "");
    assert!(!body.contains("0 adverse flag(s)"), "{body}");
    let heads: Vec<_> = history_of(&body)["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["head_sha"].clone())
        .collect();
    assert_eq!(heads, [HEAD_A, HEAD_B, HEAD_C]);
}

#[test]
fn overflow_drops_the_oldest_heads_with_notice_and_stays_within_the_host_limit() {
    let host = Host::new(HEAD_A);
    let rows = 300;
    let report = |head: &str, run: u64| {
        host.update(|state| state["head"] = json!(head));
        let reported = host.report(&ComparisonInput {
            schema_version: 1,
            current: budget_run(head, run, 1, rows, |_| ADVERSE),
            baseline: baseline(rows),
        });
        assert!(reported.delivered(), "{}", reported.diagnostics());
        assert_eq!(reported.code, Some(0));
        host.comment_body(0)
    };
    assert!(report(HEAD_A, 1).contains(HEAD_A));
    let mut body = String::new();
    for (index, head) in [HEAD_B, HEAD_C].iter().enumerate() {
        body = report(head, 2 + index as u64);
    }
    assert!(body.chars().count() <= 65_536, "{}", body.chars().count());
    // Both earlier heads were dropped; the notice sits in the collapsed section.
    let section = earlier_section(&body).unwrap_or_else(|| panic!("{body}"));
    assert!(
        section.contains("2 oldest history head(s) omitted to fit host capacity"),
        "{body}"
    );
    assert!(body.contains("Flags on earlier heads (0)"), "{body}");
    assert_eq!(flags_of(&body, HEAD_A), "", "oldest head is gone");
    assert_eq!(flags_of(&body, HEAD_B), "", "oldest head is gone");
    let history = history_of(&body);
    assert_eq!(history["omitted_heads"], 2);
    assert_eq!(history["entries"].as_array().unwrap().len(), 1);
    assert_eq!(history["entries"][0]["head_sha"], HEAD_C);
    assert_eq!(
        history["entries"][0]["flags"].as_array().unwrap().len(),
        300
    );
    assert_eq!(host.state()["comments"].as_array().unwrap().len(), 1);
}

#[test]
fn a_current_report_too_large_for_the_host_falls_back_to_a_link_and_still_delivers() {
    let host = Host::new(HEAD_A);
    let request = publish_request(HEAD_A, &"x".repeat(70_000), json!([]));
    let output = host.publish_raw(&request.to_string());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["delivered"], true, "{result}");
    let body = host.comment_body(0);
    assert!(body.contains("Complete summary/artifact: https://github.com/owner/repo"));
    assert!(body.chars().count() <= 65_536);

    // Under the host's limit by itself and over it once onevcs adds the key marker: the
    // reporter must leave room for that marker, or the host write would be refused.
    let host = Host::new(HEAD_A);
    let short = publish_request(HEAD_A, "", json!([]));
    assert_eq!(delivered(&host, &short)["delivered"], true);
    let overhead_with_marker = host.comment_body(0).len();
    let host = Host::new(HEAD_A);
    let over_with_marker = publish_request(
        HEAD_A,
        &"y".repeat(65_600 - overhead_with_marker),
        json!([]),
    );
    let result = delivered(&host, &over_with_marker);
    assert_eq!(result["delivered"], true, "{result}");
    assert!(host.comment_body(0).chars().count() <= 65_536);
}

fn delivered(host: &Host, request: &Value) -> Value {
    let output = host.publish_raw(&request.to_string());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn stale_heads_write_nothing_and_preserve_passing_and_failing_verdicts() {
    for (value, code) in [(ADVERSE, 0), (OVER, 1)] {
        let host = Host::new(HEAD_A);
        assert!(host.report(&input(HEAD_A, 1, 1, ADVERSE)).delivered());
        let before = host.comment_body(0);
        host.clear_requests();
        host.update(|state| state["head"] = json!(HEAD_B));

        let stale = host.report(&input(HEAD_A, 1, 2, value));
        assert_eq!(stale.code, Some(code), "{}", stale.report);
        assert!(!stale.delivered());
        let diagnostics = stale.diagnostics();
        assert!(diagnostics.contains(HEAD_B), "{diagnostics}");
        assert!(diagnostics.contains("no comment written"), "{diagnostics}");
        assert_eq!(host.writes(), 0);
        assert_eq!(host.comment_body(0), before);
    }
}

#[test]
fn unrelated_comments_and_descriptions_are_preserved() {
    let host = Host::new(HEAD_A);
    let marked_elsewhere = "someone else's report\n\n<!-- onevcs:standing-comment:v1:0000 -->";
    let unrelated = json!([
        {"id": 1, "node_id": "a", "html_url": "https://github.com/owner/repo/pull/1#issuecomment-1",
         "author": "someone", "body": "<!-- onebudgetspec-history\n{}\n-->"},
        {"id": 2, "node_id": "b", "html_url": "https://github.com/owner/repo/pull/1#issuecomment-2",
         "author": "viewer", "body": marked_elsewhere},
    ]);
    host.update(|state| {
        state["comments"] = unrelated.clone();
        state["description"] = json!("a description");
    });
    assert!(host.report(&input(HEAD_A, 1, 1, ADVERSE)).delivered());
    host.update(|state| state["head"] = json!(HEAD_B));
    assert!(host.report(&input(HEAD_B, 2, 1, IMPROVED)).delivered());

    let state = host.state();
    let comments = state["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 3, "exactly one owned comment joined");
    assert_eq!(comments[..2], unrelated.as_array().unwrap()[..]);
    assert_eq!(state["description"], "a description");
    assert!(comments[2]["body"].as_str().unwrap().contains(HEAD_A));
    assert!(comments[2]["body"].as_str().unwrap().contains(HEAD_B));

    // The same author's report under a different key is another comment altogether.
    let other = publish_request(HEAD_B, "another key's report", json!([]));
    let mut other_key = other;
    other_key["key"] = json!("another-key");
    let output = host.publish_raw(&other_key.to_string());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["delivered"], true);
    assert_eq!(host.state()["comments"].as_array().unwrap().len(), 4);
    assert_eq!(host.state()["comments"][2], comments[2]);
}

#[test]
fn auth_and_post_failures_answer_undelivered_and_preserve_both_budget_outcomes() {
    for (value, code) in [(ADVERSE, 0), (OVER, 1)] {
        // The read is refused: nothing is merged or written.
        let host = Host::new(HEAD_A);
        host.update(|state| state["read_status"] = json!(403));
        let refused = host.report(&input(HEAD_A, 1, 1, value));
        assert_eq!(refused.code, Some(code), "{}", refused.report);
        assert!(!refused.delivered());
        assert!(
            refused
                .diagnostics()
                .contains("could not read the standing comment"),
            "{}",
            refused.diagnostics()
        );
        assert_eq!(host.writes(), 0);
        assert_eq!(refused.report["rows"].as_array().unwrap().len(), 1);

        // No credential at all reaches the same answer.
        let host = Host::new(HEAD_A);
        let mut input_file = host.command(support::core_cli());
        let path = host.root.path().join("no-token.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&input(HEAD_A, 1, 1, value)).unwrap(),
        )
        .unwrap();
        let output = input_file
            .env_remove("GH_TOKEN")
            .args(["report", "--json", "--input"])
            .arg(&path)
            .arg("--reporter")
            .arg(support::plugin())
            .args(["--destination", CHANGE, "--key", KEY])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("report not delivered"));
        assert_eq!(host.state()["comments"], json!([]));

        // The write is refused after a good read: the diagnostic names why and stays short
        // although a failed `gh` quotes the whole body on its command line.
        let host = Host::new(HEAD_A);
        host.update(|state| state["write_status"] = json!(403));
        let failed = host.report(&ComparisonInput {
            schema_version: 1,
            current: budget_run(HEAD_A, 1, 1, 100, |_| value),
            baseline: baseline(100),
        });
        assert_eq!(failed.code, Some(code), "{}", failed.report);
        assert!(!failed.delivered());
        let diagnostics = failed.diagnostics();
        assert!(
            diagnostics.contains("could not post the standing comment"),
            "{diagnostics}"
        );
        assert!(
            diagnostics.contains("Resource not accessible"),
            "{diagnostics}"
        );
        assert_eq!(host.state()["comments"], json!([]));
        assert_eq!(failed.report["rows"].as_array().unwrap().len(), 100);
    }
}

fn publish_request(head: &str, markdown: &str, flags: Value) -> Value {
    json!({
        "schema_version": 1, "destination": CHANGE, "key": KEY, "head_sha": head,
        "run_id": "1", "run_url": "https://github.com/owner/repo/actions/runs/1",
        "markdown": markdown, "attempt": 1, "flags": flags,
    })
}

#[test]
fn imported_protocol_shapes_conform_to_the_published_schema_bundle() {
    let host = Host::new(HEAD_A);
    let flags = json!([{
        "file": "budgets.yaml", "id": "latency", "sigma_change": 2.0,
        "constant_baseline_change": false,
    }]);
    let request = publish_request(HEAD_A, "## Budget report", flags);
    assert_conforms("publish-request", &request);

    let output = host.publish_raw(&request.to_string());
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let delivered: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(delivered["delivered"], true);
    assert_eq!(
        delivered["url"],
        "https://github.com/owner/repo/pull/1#issuecomment-100"
    );
    assert_conforms("delivery-result", &delivered);

    // The history core embeds round-trips through the plugin as core's own type.
    let history = history_of(&host.comment_body(0));
    assert_conforms("report-history", &history);
    assert_eq!(history["entries"][0]["head_sha"], HEAD_A);

    for (bad, why) in [
        (
            json!({"schema_version": 1}).to_string(),
            "malformed PublishRequest",
        ),
        ("not json".to_owned(), "malformed PublishRequest"),
        (
            {
                let mut unsupported = publish_request(HEAD_A, "x", json!([]));
                unsupported["schema_version"] = json!(2);
                unsupported.to_string()
            },
            "schema_version 1",
        ),
        (
            publish_request("not-a-commit", "x", json!([])).to_string(),
            "head_sha",
        ),
        (
            {
                let mut elsewhere = publish_request(HEAD_A, "x", json!([]));
                elsewhere["destination"] = json!("https://example.test/not-a-change");
                elsewhere.to_string()
            },
            "could not read the standing comment",
        ),
    ] {
        host.clear_requests();
        let output = host.publish_raw(&bad);
        assert!(output.status.success(), "{output:?}");
        let refused: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_conforms("delivery-result", &refused);
        assert_eq!(refused["delivered"], false, "{refused}");
        assert!(
            refused["diagnostic"].as_str().unwrap().contains(why),
            "{refused}"
        );
        assert_eq!(
            host.paths(),
            Vec::<String>::new(),
            "refused before the host"
        );
    }
}
