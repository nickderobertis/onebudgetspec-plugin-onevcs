//! The installed `history` verb, real onevcs and real git, with only GitHub's HTTP doubled.
#![cfg(unix)]
mod support;

use onebudgetspec_core::comparison::{BudgetRun, ComparisonInput, RollingHistory};
use serde_json::json;
use support::{
    ARTIFACT, HistoryAnswer, Host, ROLLING_FILE, Repository, archive, assert_conforms, budget_run,
    carried_forward, rolling, rolling_archive,
};

/// A base-branch job as the action runs it: read, carry forward, then upload unless the
/// upload is to fail. Run and artifact ids come from the commit's own order.
struct BaseJobs<'a> {
    host: &'a Host,
    repository: &'a Repository,
    rows: usize,
    next: std::cell::Cell<u64>,
}

impl<'a> BaseJobs<'a> {
    fn new(host: &'a Host, repository: &'a Repository, rows: usize) -> Self {
        Self {
            host,
            repository,
            rows,
            next: std::cell::Cell::new(1),
        }
    }

    fn id(&self) -> u64 {
        let id = self.next.get();
        self.next.set(id + 1);
        id
    }

    fn entry(&self, head: &str, run: u64, attempt: u32) -> BudgetRun {
        budget_run(head, run, attempt, self.rows, |row| {
            10.0 + (run as f64) + (row as f64) / 100.0
        })
    }

    /// Read what this commit's job would read, as it is before the job's own run exists.
    fn read(&self, head: &str) -> Option<RollingHistory> {
        let answer = self.host.history(self.repository.path(), head);
        answer.history.is_some().then(|| answer.parsed())
    }

    fn run(&self, head: &str, uploaded: bool) -> u64 {
        let previous = self.read(head);
        self.finish(head, previous, uploaded)
    }

    fn finish(&self, head: &str, previous: Option<RollingHistory>, uploaded: bool) -> u64 {
        let run = self.id();
        let history = carried_forward(previous, self.entry(head, run, 1));
        self.host.add_run(run, head, 1, run as u32);
        if uploaded {
            self.host
                .upload(run, 1000 + run, ARTIFACT, &rolling_archive(&history));
        }
        run
    }
}

fn answer_after(host: &Host, repository: &Repository, head: &str) -> HistoryAnswer {
    host.clear_requests();
    let answer = host.history(repository.path(), head);
    assert_eq!(
        answer.report["cost"]["rest_requests"].as_u64().unwrap() as usize,
        host.paths().len(),
        "QueryCost counts the requests the host saw: {}",
        answer.report
    );
    assert_eq!(answer.report["cost"]["graphql_points"], 0);
    answer
}

#[test]
fn twenty_commits_of_100_rows_conform_and_feed_real_core_comparison() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 100);
    let mut heads = Vec::new();
    for _ in 0..22 {
        let head = repository.commit();
        jobs.run(&head, true);
        heads.push(head);
    }
    let base = repository.commit();

    let answer = answer_after(&host, &repository, &base);
    assert_eq!(answer.code, Some(0), "{}", answer.stderr);
    assert!(answer.stderr.is_empty(), "{}", answer.stderr);
    assert_eq!(
        answer.heads(),
        heads[2..],
        "the oldest two were trimmed, in ancestry order"
    );
    assert_eq!(
        answer.disclosed("lost"),
        Vec::<String>::new(),
        "trimmed is not lost"
    );
    let newest = heads.last().unwrap();
    assert_eq!(answer.report["status"], "available");
    assert_eq!(answer.report["selected"]["head_sha"], newest.as_str());
    assert_eq!(answer.report["selected"]["entries"], 20);
    let requests = host.paths();
    assert_eq!(
        requests.len(),
        3,
        "one listing, one artifact list, one download: {requests:?}"
    );
    assert!(
        requests[0].contains("/actions/workflows/42/runs?"),
        "{requests:?}"
    );
    assert!(requests[0].contains("branch=main&event=push&status=completed&per_page=20"));

    // The saved file is the authoritative contract, holding the ordinary rows unchanged.
    let history = answer.history.as_ref().unwrap();
    assert_conforms("rolling-history", history);
    let parsed = answer.parsed();
    assert_eq!(parsed.runs.len(), 20);
    assert!(parsed.runs.iter().all(|run| run.catalog.len() == 100));
    assert_eq!(parsed.runs[19], jobs.entry(newest, 22, 1));

    // Its actual output feeds real core with no rejected entry.
    let pull_head = repository.divergent_commit(&base);
    host.update(|state| state["head"] = json!(pull_head));
    let reported = host.report(&ComparisonInput {
        schema_version: 1,
        current: budget_run(&pull_head, 900, 1, 100, |row| 40.0 + row as f64 / 10.0),
        baseline: parsed.runs,
    });
    assert_eq!(reported.code, Some(0), "{}", reported.report);
    assert!(reported.delivered(), "{}", reported.diagnostics());
    let rows = reported.report["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 100);
    assert!(rows.iter().all(|row| row["baseline_n"] == 20));
    assert!(rows.iter().all(|row| row["flag"] == true));
    let diagnostics = reported.report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 20, "{diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .all(|d| d.as_str().unwrap().starts_with("baseline provenance"))
    );
    let body = host.comment_body(0);
    assert!(body.contains("| budgets.yaml / budget-000 (ms) | within | 40.000"));
    assert!(
        !body.contains("provisional"),
        "twenty samples are not provisional"
    );
}

#[test]
fn a_failed_upload_leaves_the_prior_artifact_and_discloses_the_lost_entry() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);
    let first = repository.commit();
    jobs.run(&first, true);
    let second = repository.commit();
    jobs.run(&second, true);
    let failed = repository.commit();
    jobs.run(&failed, false);
    let after = repository.commit();

    let answer = answer_after(&host, &repository, &after);
    assert_eq!(answer.code, Some(0), "{}", answer.stderr);
    assert_eq!(answer.heads(), [first.clone(), second.clone()]);
    assert_eq!(host.paths().len(), 4, "{:?}", host.paths());
    assert!(
        answer
            .reason_for("skipped", &failed)
            .contains("no artifact named")
    );
    assert!(
        answer
            .reason_for("lost", &failed)
            .contains("not in the selected history")
    );

    // The next job carries on past the loss, which later readers still see disclosed.
    jobs.run(&after, true);
    let later = repository.commit();
    let answer = answer_after(&host, &repository, &later);
    assert_eq!(answer.heads(), [first, second, after]);
    assert_eq!(answer.disclosed("lost"), [failed]);
    assert_eq!(answer.disclosed("skipped"), Vec::<String>::new());
    assert_eq!(host.paths().len(), 3);
}

#[test]
fn concurrent_pushes_pick_by_ancestry_never_merge_and_exclude_divergent_runs() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);
    let root = repository.commit();
    jobs.run(&root, true);

    // Two pushes in quick succession both read the same earlier file.
    let middle = repository.commit();
    let newer = repository.commit();
    let previous_of_middle = jobs.read(&middle);
    let previous_of_newer = jobs.read(&newer);
    let newer_run = jobs.finish(&newer, previous_of_newer, true);
    // The older commit's job finishes last and is created later: timestamps must not win.
    let middle_run = jobs.finish(&middle, previous_of_middle, true);
    assert!(middle_run > newer_run);
    // A force-pushed line is the newest thing on the branch, and is no ancestor of the base.
    let forked = repository.divergent_commit(&root);
    let forked_run = jobs.id();
    host.add_run(forked_run, &forked, 1, 500);
    let unrelated = carried_forward(None, jobs.entry(&forked, forked_run, 1));
    host.upload(forked_run, 2000, ARTIFACT, &rolling_archive(&unrelated));

    let base = repository.commit();
    let answer = answer_after(&host, &repository, &base);
    assert_eq!(answer.code, Some(0), "{}", answer.stderr);
    assert_eq!(answer.report["selected"]["run_id"], newer_run.to_string());
    assert_eq!(
        answer.heads(),
        [root.clone(), newer.clone()],
        "the files are never merged"
    );
    assert!(
        answer
            .reason_for("lost", &middle)
            .contains("not in the selected history")
    );
    assert_eq!(answer.disclosed("lost"), [middle]);
    assert!(
        answer
            .reason_for("skipped", &forked)
            .contains("not reachable")
    );
    assert!(
        !host
            .paths()
            .iter()
            .any(|path| path.contains(&format!("/runs/{forked_run}/"))),
        "a divergent run's artifacts are never read: {:?}",
        host.paths()
    );
    assert_eq!(host.paths().len(), 3);

    // Read from the divergent line itself, the other lineage is what is unrelated.
    let answer = answer_after(&host, &repository, &forked);
    assert_eq!(answer.heads(), vec![forked.clone()]);
    assert!(
        answer
            .reason_for("skipped", &newer)
            .contains("not reachable")
    );
}

#[test]
fn reruns_use_the_latest_attempt_once_and_refuse_a_file_from_an_earlier_one() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);
    let first = repository.commit();
    jobs.run(&first, true);
    let second = repository.commit();
    let previous = jobs.read(&second);
    let second_run = jobs.finish(&second, previous.clone(), true);
    let base = repository.commit();

    // Attempt 2 uploads another immutable artifact of the same name beside attempt 1's.
    host.rerun(second_run, 2);
    let rerun = carried_forward(previous.clone(), jobs.entry(&second, second_run, 2));
    host.upload(second_run, 1100, ARTIFACT, &rolling_archive(&rerun));
    let answer = answer_after(&host, &repository, &base);
    assert_eq!(answer.code, Some(0), "{}", answer.stderr);
    assert_eq!(answer.report["selected"]["artifact_id"], "1100");
    assert_eq!(answer.report["selected"]["attempt"], 2);
    let attempts: Vec<_> = answer.parsed().runs.iter().map(|run| run.attempt).collect();
    assert_eq!(attempts, [1, 2], "one entry per commit, the latest attempt");
    assert_eq!(answer.heads(), [first.clone(), second.clone()]);

    // Attempt 3 failed to upload: attempt 2's file no longer matches the host's latest attempt.
    host.rerun(second_run, 3);
    let answer = answer_after(&host, &repository, &base);
    assert_eq!(answer.code, Some(0), "{}", answer.stderr);
    assert_eq!(answer.heads(), vec![first.clone()]);
    assert!(
        answer
            .reason_for("skipped", &second)
            .contains("latest attempt 3")
    );
    assert!(
        answer
            .reason_for("lost", &second)
            .contains("not in the selected history")
    );

    // A rerun of a commit a newer file already carries: that file holds the older attempt,
    // so the newer attempt is disclosed rather than silently absent.
    let carried = repository.commit();
    let carried_run = jobs.run(&carried, true);
    let child = repository.commit();
    jobs.run(&child, true);
    host.rerun(carried_run, 2);
    let answer = answer_after(&host, &repository, &child);
    assert_eq!(
        answer.report["selected"]["head_sha"],
        child.as_str(),
        "{}",
        answer.report
    );
    assert!(
        answer
            .reason_for("lost", &carried)
            .contains("holds attempt 1 of this commit; attempt 2 did not reach it"),
        "{}",
        answer.report
    );

    // A later run re-executing a known commit under a new run id: its file wins, once, and
    // the earlier run of that commit is no loss.
    let third = repository.commit();
    let held = jobs.read(&third).unwrap();
    assert_eq!(held.runs.len(), 3);
    let third_run = jobs.finish(&third, Some(held.clone()), true);
    let again = jobs.finish(&third, Some(held), true);
    assert!(again > third_run);
    let answer = answer_after(&host, &repository, &repository.commit());
    assert_eq!(answer.report["selected"]["run_id"], again.to_string());
    assert_eq!(answer.heads(), [first, carried, child, third]);
    assert!(!answer.disclosed("lost").iter().any(|head| head == &base));
}

#[test]
fn wrong_branch_workflow_event_and_incomplete_runs_are_never_read() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);
    let head = repository.commit();
    let good = jobs.run(&head, true);
    let base = repository.commit();

    let decoys = [
        json!({"head_branch": "feature"}),
        json!({"workflow_id": 7}),
        json!({"event": "pull_request"}),
        json!({"status": "in_progress", "conclusion": null}),
    ];
    for (index, facts) in decoys.iter().enumerate() {
        let id = 700 + index as u64;
        let mut run = facts.clone();
        run["id"] = json!(id);
        run["head_sha"] = json!(base);
        run["created_at"] = json!("2026-11-02T00:00:00Z");
        host.add_listed_run(run);
        let decoy = carried_forward(None, jobs.entry(&base, id, 1));
        host.upload(id, 3000 + id, ARTIFACT, &rolling_archive(&decoy));
    }

    let answer = answer_after(&host, &repository, &base);
    assert_eq!(answer.code, Some(0), "{}", answer.stderr);
    assert_eq!(answer.report["selected"]["run_id"], good.to_string());
    assert_eq!(answer.heads(), [head]);
    let paths = host.paths();
    assert!(
        !paths.iter().any(|path| path.contains("/runs/70")),
        "{paths:?}"
    );
    assert_eq!(paths.len(), 3);

    // With only decoys listed there is no history, and the reason says what was looked for.
    host.update(|state| {
        let runs = state["runs"].as_array_mut().unwrap();
        runs.retain(|run| run["id"].as_u64().unwrap() >= 700);
    });
    let answer = answer_after(&host, &repository, &base);
    assert_eq!(answer.code, Some(1));
    assert!(answer.history.is_none());
    assert!(
        answer
            .stderr
            .contains("0 completed push run(s) of workflow 42 on main"),
        "{}",
        answer.stderr
    );
    assert_eq!(answer.report["status"], "unavailable");
    assert_eq!(host.paths().len(), 1);
}

/// Every embedded commit and identity is checked; a bad newest file falls back to the
/// older good run rather than letting any part of it through.
#[test]
fn invalid_or_unrelated_files_are_refused_whole_and_nothing_unrelated_reaches_comparison() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);
    let first = repository.commit();
    jobs.run(&first, true);
    let middle = repository.commit();
    jobs.run(&middle, true);
    let good_heads = [first.clone(), middle.clone()];
    let head = repository.commit();
    let run = 50;
    host.add_run(run, &head, 1, 400);
    let base = repository.commit();
    let fork = repository.divergent_commit(&first);
    let previous = jobs.read(&base).unwrap();

    let entry = || jobs.entry(&head, run, 1);
    let good = carried_forward(Some(previous.clone()), entry());
    let tampered = |change: &dyn Fn(&mut RollingHistory)| {
        let mut history = good.clone();
        change(&mut history);
        rolling_archive(&history)
    };
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("not an archive", b"not a zip".to_vec(), "not a readable archive"),
        ("no rolling member", archive(&[("budget-run.json", b"{}")]), "has no rolling-history.json"),
        ("not json", archive(&[(ROLLING_FILE, b"{")]), "not a rolling history"),
        (
            "unknown field",
            archive(&[(ROLLING_FILE, br#"{"schema_version":1,"repository":"owner/repo","base_ref":"main","runs":[],"extra":1}"#)]),
            "not a rolling history",
        ),
        ("empty", rolling_archive(&rolling(Vec::new())), "holds no runs"),
        ("schema version", tampered(&|h| h.schema_version = 2), "schema_version 1"),
        ("wrong repository", tampered(&|h| {
            h.repository = "other/repo".into();
            for run in &mut h.runs { run.repository = "other/repo".into(); }
        }), "names other/repo on main"),
        ("wrong branch", tampered(&|h| {
            h.base_ref = "feature".into();
            for run in &mut h.runs { run.base_ref = "feature".into(); }
        }), "names owner/repo on feature"),
        ("wrong run id", tampered(&|h| h.runs.last_mut().unwrap().run_id = "9999".into()), "not this run's latest attempt"),
        ("wrong attempt", tampered(&|h| h.runs.last_mut().unwrap().attempt = 2), "not this run's latest attempt"),
        ("wrong head", tampered(&|h| h.runs.last_mut().unwrap().head_sha = middle.clone()), "duplicate head"),
        ("incomplete run", tampered(&|h| h.runs[0].attempt = 0), "positive attempt"),
        ("duplicate commit", tampered(&|h| {
            let copy = h.runs[0].clone();
            h.runs.insert(1, copy);
        }), "duplicate head"),
        ("over twenty", tampered(&|h| {
            let template = h.runs[0].clone();
            for n in 0..20 {
                let mut extra = template.clone();
                extra.head_sha = format!("{n:040x}");
                h.runs.insert(0, extra);
            }
        }), "at most 20 runs"),
        ("abbreviated commit", tampered(&|h| h.runs[0].head_sha = "abc".into()), "embedded head_sha"),
        ("divergent commit", tampered(&|h| h.runs.insert(0, jobs.entry(&fork, 60, 1))), "not an ancestor"),
        ("unknown commit", tampered(&|h| {
                h.runs[0].head_sha = "1".repeat(40);
                h.runs[0].run_id = "61".into();
            }), "not an ancestor"),
        ("out of order", tampered(&|h| h.runs.swap(0, 1)), "not an ancestor"),
        ("contradicts the host", tampered(&|h| h.runs[0].head_sha = base.clone()), "contradicts the host"),
    ];
    for (name, bytes, why) in cases {
        let archive_id = 5000;
        host.update(|state| state["artifacts"][run.to_string()] = json!([]));
        host.upload(run, archive_id, ARTIFACT, &bytes);
        let answer = answer_after(&host, &repository, &base);
        assert_eq!(answer.code, Some(0), "{name}: {}", answer.stderr);
        assert_eq!(answer.heads(), good_heads, "{name}");
        let reason = answer.reason_for("skipped", &head);
        assert!(reason.contains(why), "{name}: {reason}");
        assert_eq!(answer.disclosed("lost"), vec![head.clone()], "{name}");
        assert_eq!(host.paths().len(), 5, "{name}: {:?}", host.paths());
    }

    // The same file, unaltered, is the control: it is accepted, so each refusal above is
    // the tampering's and not the fixture's.
    host.update(|state| state["artifacts"][run.to_string()] = json!([]));
    host.upload(run, 5001, ARTIFACT, &rolling_archive(&good));
    let accepted = answer_after(&host, &repository, &base);
    assert_eq!(accepted.heads(), [first, middle, head]);
}

#[test]
fn missing_expired_and_failed_reads_are_explicitly_unavailable_within_the_request_bound() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);

    // First ever run: nothing listed.
    let first = repository.commit();
    let answer = answer_after(&host, &repository, &first);
    assert_eq!(answer.code, Some(1));
    assert_eq!(answer.report["status"], "unavailable");
    assert!(answer.history.is_none());
    assert_eq!(
        answer.report["reason"].as_str().unwrap(),
        answer
            .stderr
            .trim()
            .strip_prefix("error: history unavailable: ")
            .unwrap()
    );

    // An expired artifact is named as such and nothing is downloaded.
    let run = jobs.id();
    host.add_run(run, &first, 1, 1);
    host.attach(run, 6000, ARTIFACT, 10, true);
    let answer = answer_after(&host, &repository, &first);
    assert_eq!(answer.code, Some(1));
    assert!(
        answer.reason_for("skipped", &first).contains("has expired"),
        "{}",
        answer.report
    );
    assert_eq!(host.paths().len(), 2);

    // An artifact far too large is refused before it is downloaded.
    host.update(|state| state["artifacts"][run.to_string()] = json!([]));
    host.attach(run, 6001, ARTIFACT, 65 * 1024 * 1024, false);
    let answer = answer_after(&host, &repository, &first);
    assert!(
        answer.reason_for("skipped", &first).contains("byte limit"),
        "{}",
        answer.report
    );
    assert_eq!(host.paths().len(), 2);

    // Another artifact name is not this history.
    host.update(|state| state["artifacts"][run.to_string()] = json!([]));
    host.attach(run, 6002, "something-else", 10, false);
    let answer = answer_after(&host, &repository, &first);
    assert!(
        answer
            .reason_for("skipped", &first)
            .contains("no artifact named budget-results")
    );

    // A listed archive the host no longer serves.
    host.update(|state| state["artifacts"][run.to_string()] = json!([]));
    host.attach(run, 6003, ARTIFACT, 10, false);
    let answer = answer_after(&host, &repository, &first);
    assert_eq!(answer.code, Some(1));
    assert!(
        answer
            .reason_for("skipped", &first)
            .contains("could not download"),
        "{}",
        answer.report
    );

    // Three bad runs: the third is disclosed unread rather than walked toward.
    let mut heads = vec![first.clone()];
    for minute in 2..5 {
        let head = repository.commit();
        let id = jobs.id();
        host.add_run(id, &head, 1, minute);
        host.attach(id, 6100 + id, ARTIFACT, 10, false);
        heads.push(head);
    }
    let base = heads.last().unwrap().clone();
    let answer = answer_after(&host, &repository, &base);
    assert_eq!(answer.code, Some(1));
    assert_eq!(host.paths().len(), 5, "{:?}", host.paths());
    let downloads = host.paths().iter().filter(|p| p.ends_with("/zip")).count();
    assert_eq!(downloads, 2);
    assert!(
        answer
            .reason_for("skipped", &heads[1])
            .contains("request bound"),
        "{}",
        answer.report
    );
    assert!(
        answer
            .reason_for("skipped", &heads[0])
            .contains("request bound"),
        "{}",
        answer.report
    );

    // Bad credentials: every failed request is still counted, and no file appears.
    let no_token = |host: &Host| {
        let run = host
            .command(support::plugin())
            .env_remove("GH_TOKEN")
            .current_dir(repository.path())
            .args([
                "history",
                "--repo",
                "owner/repo",
                "--base-ref",
                "main",
                "--base-sha",
                &first,
            ])
            .args([
                "--workflow",
                "42",
                "--artifact-name",
                ARTIFACT,
                "--output",
                "absent-file.json",
            ])
            .output()
            .unwrap();
        assert!(!repository.path().join("absent-file.json").exists());
        run
    };
    host.clear_requests();
    let refused = no_token(&host);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("set GH_TOKEN or GITHUB_TOKEN"));
    assert_eq!(
        host.paths().len(),
        0,
        "no request is made without a credential"
    );
}

#[test]
fn listing_failures_git_failures_and_output_failures_are_diagnostic() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);
    let head = repository.commit();
    jobs.run(&head, true);
    let base = repository.commit();

    // A stale file from another read cannot pass for this read's answer.
    let output = host.root.path().join("stale.json");
    std::fs::write(&output, "stale").unwrap();
    let unknown = "1".repeat(40);
    let answer = host.run_history(repository.path(), &unknown, &output);
    assert_eq!(answer.code, Some(1));
    assert!(
        answer.stderr.contains("is not available locally"),
        "{}",
        answer.stderr
    );
    assert!(!output.exists());
    assert_eq!(
        answer.report["cost"]["rest_requests"], 0,
        "git refuses before the host"
    );

    // Not a checkout at all.
    let elsewhere = tempfile::tempdir().unwrap();
    host.clear_requests();
    let answer = host.history(elsewhere.path(), &base);
    assert_eq!(answer.code, Some(1));
    assert!(
        answer.stderr.contains("inside the base checkout"),
        "{}",
        answer.stderr
    );
    assert_eq!(host.paths().len(), 0);

    // A shallow clone cannot prove ancestry, however good the file.
    let shallow = tempfile::tempdir().unwrap();
    let source = format!("file://{}", repository.path().display());
    let cloned = std::process::Command::new("git")
        .args(["clone", "--quiet", "--depth", "1", &source])
        .arg(shallow.path())
        .output()
        .unwrap();
    assert!(cloned.status.success(), "{cloned:?}");
    let answer = host.history(shallow.path(), &base);
    assert_eq!(answer.code, Some(1));
    assert!(answer.stderr.contains("shallow clone"), "{}", answer.stderr);

    // A run whose commit this checkout does not hold is unproven, never assumed.
    let unfetched = "2".repeat(40);
    host.add_run(77, &unfetched, 1, 900);
    let answer = answer_after(&host, &repository, &base);
    assert_eq!(answer.code, Some(0), "{}", answer.stderr);
    assert!(
        answer
            .reason_for("skipped", &unfetched)
            .contains("ancestry unproven")
    );
    assert_eq!(answer.heads(), vec![head.clone()]);

    // The host refuses the listing.
    let answer = {
        host.update(|state| state["runs"] = json!("malformed"));
        answer_after(&host, &repository, &base)
    };
    assert_eq!(answer.code, Some(1));
    assert!(
        answer.stderr.contains("could not list base runs"),
        "{}",
        answer.stderr
    );
    host.update(|state| state["runs"] = json!([]));

    // Output that cannot be written, with a host that would answer.
    host.add_run(1, &head, 1, 1);
    let missing = host.root.path().join("missing/rolling.json");
    let answer = host.run_history(repository.path(), &base, &missing);
    assert_eq!(answer.code, Some(1));
    assert!(
        answer.stderr.contains("does not exist"),
        "{}",
        answer.stderr
    );
    let directory = host.root.path().join("a-directory");
    std::fs::create_dir(&directory).unwrap();
    let answer = host.run_history(repository.path(), &base, &directory);
    assert_eq!(answer.code, Some(1));
    assert!(
        answer.stderr.contains("choose a writable --output"),
        "{}",
        answer.stderr
    );

    // Usage errors perform no work and leave the host alone.
    host.clear_requests();
    for args in [
        [
            "--repo",
            "not a slug",
            "--base-sha",
            &base,
            "--workflow",
            "1",
        ],
        [
            "--repo",
            "owner/repo",
            "--base-sha",
            "short",
            "--workflow",
            "1",
        ],
        [
            "--repo",
            "owner/repo",
            "--base-sha",
            &base,
            "--workflow",
            "0",
        ],
    ] {
        let usage = host
            .command(support::plugin())
            .current_dir(repository.path())
            .arg("history")
            .args(args)
            .args([
                "--base-ref",
                "main",
                "--artifact-name",
                ARTIFACT,
                "--output",
                "x.json",
            ])
            .output()
            .unwrap();
        assert_eq!(usage.status.code(), Some(2), "{usage:?}");
    }
    for (base_ref, name) in [("refs/heads/main", ARTIFACT), ("main", " ")] {
        let usage = host
            .command(support::plugin())
            .current_dir(repository.path())
            .args([
                "history",
                "--repo",
                "owner/repo",
                "--workflow",
                "1",
                "--base-sha",
                &base,
            ])
            .args([
                "--base-ref",
                base_ref,
                "--artifact-name",
                name,
                "--output",
                "x.json",
            ])
            .output()
            .unwrap();
        assert_eq!(usage.status.code(), Some(1), "{usage:?}");
        assert!(String::from_utf8_lossy(&usage.stderr).contains("bare branch name"));
    }
    assert_eq!(host.paths().len(), 0);
}

#[test]
fn a_listed_commit_older_than_a_full_history_was_trimmed_not_lost() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);
    let mut heads = Vec::new();
    for _ in 0..21 {
        let head = repository.commit();
        jobs.run(&head, true);
        heads.push(head);
    }
    // The oldest commit's run stays on the listed page, as a rerun's recent creation would keep it.
    host.update(|state| state["runs"][0]["created_at"] = json!("2026-11-02T00:00:00Z"));
    let answer = answer_after(&host, &repository, heads.last().unwrap());
    assert_eq!(answer.heads(), heads[1..]);
    assert_eq!(
        answer.disclosed("lost"),
        Vec::<String>::new(),
        "{}",
        answer.report
    );
}

#[test]
fn the_request_bound_also_stops_a_download_and_listing_failures_are_counted() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);
    let good = repository.commit();
    jobs.run(&good, true);
    let mut failed = Vec::new();
    for _ in 0..3 {
        let head = repository.commit();
        jobs.run(&head, false);
        failed.push(head);
    }
    // One listing and four artifact lists spend the bound before the good file's download.
    let answer = answer_after(&host, &repository, failed.last().unwrap());
    assert_eq!(answer.code, Some(1), "{}", answer.report);
    assert!(
        answer
            .stderr
            .contains("limit of 5 REST requests was reached"),
        "{}",
        answer.stderr
    );
    assert_eq!(host.paths().len(), 5, "{:?}", host.paths());
    assert!(
        answer
            .reason_for("skipped", &good)
            .contains("not downloaded"),
        "{}",
        answer.report
    );

    // A refused artifact listing still reached the host, and the cost says so.
    host.update(|state| state["list_status"] = json!(500));
    let answer = answer_after(&host, &repository, &good);
    assert_eq!(answer.code, Some(1), "{}", answer.report);
    assert!(
        answer
            .reason_for("skipped", &good)
            .contains("could not list its artifacts"),
        "{}",
        answer.report
    );
    assert_eq!(host.paths().len(), 2);
}

#[test]
fn an_oversized_rolling_member_is_refused_without_reading_it_all() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let head = repository.commit();
    host.add_run(1, &head, 1, 1);
    let padding = vec![b' '; 65 * 1024 * 1024];
    host.upload(1, 7000, ARTIFACT, &archive(&[(ROLLING_FILE, &padding)]));
    let answer = answer_after(&host, &repository, &head);
    assert_eq!(answer.code, Some(1), "{}", answer.report);
    assert!(
        answer.reason_for("skipped", &head).contains("byte limit"),
        "{}",
        answer.report
    );
}

#[test]
fn an_output_that_cannot_be_written_leaves_no_file_and_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);
    let head = repository.commit();
    jobs.run(&head, true);
    let locked = host.root.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let path = locked.join("rolling.json");
    let answer = host.run_history(repository.path(), &head, &path);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(answer.code, Some(1), "{}", answer.report);
    assert!(
        answer.stderr.contains("choose a writable --output"),
        "{}",
        answer.stderr
    );
    assert_eq!(answer.report["status"], "unavailable");
    assert_eq!(
        answer.report["cost"]["rest_requests"], 3,
        "the read's cost is still reported"
    );
    assert!(!path.exists());
}

/// The fallback skips a run with no usable file and stops at the first valid one: an older
/// valid file holding a commit the chosen one lacks is neither read nor combined.
#[test]
fn the_fallback_uses_exactly_one_file_the_first_valid_one() {
    let repository = Repository::new();
    let host = Host::new(&"0".repeat(40));
    let jobs = BaseJobs::new(&host, &repository, 1);
    let oldest = repository.commit();
    let oldest_run = jobs.run(&oldest, true);
    // Started fresh, as after an unavailable baseline, so `oldest` is only in the older file.
    let chosen = repository.commit();
    let chosen_run = jobs.finish(&chosen, None, true);
    let skipped = repository.commit();
    jobs.run(&skipped, false);

    let answer = answer_after(&host, &repository, &skipped);
    assert_eq!(answer.code, Some(0), "{}", answer.stderr);
    assert!(
        answer
            .reason_for("skipped", &skipped)
            .contains("no artifact named")
    );
    assert_eq!(answer.report["selected"]["run_id"], chosen_run.to_string());
    assert_eq!(
        answer.heads(),
        vec![chosen.clone()],
        "only the chosen file's content"
    );
    let paths = host.paths();
    assert_eq!(
        paths.iter().filter(|p| p.ends_with("/zip")).count(),
        1,
        "{paths:?}"
    );
    assert_eq!(paths.len(), 4, "{paths:?}");
    assert!(
        !paths
            .iter()
            .any(|p| p.contains(&format!("/runs/{oldest_run}/"))),
        "the older valid run is never read: {paths:?}"
    );
    assert_eq!(answer.disclosed("lost"), [skipped, oldest]);
}
