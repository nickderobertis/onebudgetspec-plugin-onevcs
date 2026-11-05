//! One saved `RollingHistory`, read from one base-branch run's immutable artifact.
//!
//! Host facts select a run; the file's own entries must then agree with that run, and
//! local git must prove every embedded commit reaches the requested base head. A file
//! that fails any of it is refused whole rather than filtered: losing an entry is
//! acceptable, and an unrelated entry is not.
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use onebudgetspec_core::comparison::{RollingHistory, validate_history};
use onevcs::host_query::{
    self, ArtifactRequest, CommitSha, HostArtifact, PositiveId, QueryCost, RepositorySlug, Run,
    RunArtifactsRequest, RunsRequest,
};
use serde::Serialize;

use crate::ancestry::{Checkout, Relation};

/// The rolling file's name at the root of the artifact the base job uploads.
pub const ROLLING_HISTORY_FILE: &str = "rolling-history.json";

/// The one listing page this reads; older runs are never walked.
const LISTED_RUNS: u32 = 20;
/// The whole read's REST bound: a listing and two runs' artifact list and download, so a
/// failed upload can fall back to the run before it without reconstructing anything.
const MAX_REST_REQUESTS: u32 = 5;
/// Refuses an archive or member far beyond 20 commits of 100 rows before reading it.
const MAX_SAVED_BYTES: u64 = 64 * 1024 * 1024;
/// What onevcs's host queries authenticate with, checked first because a query refused
/// for lacking one reaches no host and so costs no request.
const CREDENTIALS: [&str; 2] = ["GITHUB_TOKEN", "GH_TOKEN"];
/// The largest a rolling file can be, from `RollingHistory`'s own contract.
const MAX_ROLLING_RUNS: usize = 20;

/// What `history` asks for.
#[derive(Debug, Clone)]
pub struct HistoryRequest {
    /// The repository as `OWNER/NAME`, which every entry must name.
    pub repository: RepositorySlug,
    /// The exact base branch, which every entry must name.
    pub base_ref: String,
    /// The base head every embedded commit must reach.
    pub base_sha: CommitSha,
    /// The workflow whose push runs are read.
    pub workflow: PositiveId,
    /// The artifact holding [`ROLLING_HISTORY_FILE`].
    pub artifact_name: String,
    /// The full-history checkout that proves ancestry.
    pub checkout: PathBuf,
}

/// The run and artifact a history came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectedArtifact {
    /// The producing run.
    pub run_id: String,
    /// The commit it ran at.
    pub head_sha: String,
    /// Its latest attempt, which produced the file.
    pub attempt: u64,
    /// The immutable artifact read.
    pub artifact_id: String,
    /// How many base commits the file holds.
    pub entries: usize,
}

/// A listed run, and why it did not contribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Disclosure {
    /// The host's run id.
    pub run_id: String,
    /// The commit it ran at.
    pub head_sha: String,
    /// Why.
    pub reason: String,
}

/// How the read ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Resolution {
    /// A history whose provenance was proven.
    Available {
        /// Where it came from.
        selected: SelectedArtifact,
        /// The history, which is written to the output file rather than to the report.
        #[serde(skip)]
        history: RollingHistory,
    },
    /// No history: the caller's baseline is unavailable, or its base history starts fresh.
    Unavailable {
        /// Why.
        reason: String,
    },
}

/// What one read did, written as JSON on standard output whichever way it ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryReport {
    /// Always `1`.
    pub schema_version: u32,
    /// How it ended.
    #[serde(flatten)]
    pub resolution: Resolution,
    /// Listed runs refused or not read, with the reason.
    pub skipped: Vec<Disclosure>,
    /// Reachable base runs whose entries the history does not hold.
    pub lost: Vec<Disclosure>,
    /// Every REST request made, including one that failed.
    pub cost: QueryCost,
}

impl HistoryReport {
    fn unavailable(reason: String, spent: Spent) -> Self {
        Self {
            schema_version: 1,
            resolution: Resolution::Unavailable { reason },
            skipped: spent.skipped,
            lost: Vec::new(),
            cost: spent.cost,
        }
    }
}

/// Read the history and write it to `output`; `output` afterwards holds this read's
/// history or does not exist, so a stale file cannot pass for this read's answer.
#[must_use]
pub fn read_to_file(request: &HistoryRequest, output: &Path) -> HistoryReport {
    if let Err(reason) = clear_output(output) {
        return HistoryReport::unavailable(reason, Spent::default());
    }
    let mut report = read(request);
    if let Resolution::Available { history, .. } = &report.resolution
        && let Err(reason) = write_output(history, output)
    {
        report.resolution = Resolution::Unavailable { reason };
    }
    report
}

/// Read and validate one history, without writing it anywhere.
#[must_use]
pub fn read(request: &HistoryRequest) -> HistoryReport {
    let mut spent = Spent::default();
    if request.base_ref.trim().is_empty()
        || request.base_ref.starts_with("refs/")
        || request.artifact_name.trim().is_empty()
    {
        return HistoryReport::unavailable(
            "--base-ref must be a bare branch name and --artifact-name must not be blank".into(),
            spent,
        );
    }
    if !CREDENTIALS
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
    {
        return HistoryReport::unavailable(
            "no GitHub credential: set GH_TOKEN or GITHUB_TOKEN to a token with actions: read"
                .into(),
            spent,
        );
    }
    let checkout = match Checkout::open(&request.checkout, &request.base_sha) {
        Ok(checkout) => checkout,
        Err(reason) => return HistoryReport::unavailable(reason, spent),
    };
    let runs = match spent.list_runs(request) {
        Ok(runs) => runs,
        Err(reason) => return HistoryReport::unavailable(reason, spent),
    };
    let candidates = spent.reachable(&checkout, &runs);
    let mut chosen = None;
    for (position, run) in candidates.iter().enumerate() {
        if spent.cost.rest_requests >= MAX_REST_REQUESTS {
            for unread in &candidates[position..] {
                spent.skip(
                    unread,
                    "not read: this read's request bound was reached, and older artifacts are never walked",
                );
            }
            break;
        }
        match spent.read_candidate(request, &checkout, &runs, run) {
            Ok(found) => {
                chosen = Some(found);
                break;
            }
            Err(reason) => spent.skip(run, &reason),
        }
    }
    let Some((history, selected)) = chosen else {
        let reason = format!(
            "no eligible rolling history among {} completed push run(s) of workflow {} on {}; see skipped",
            runs.len(),
            request.workflow.get(),
            request.base_ref
        );
        return HistoryReport::unavailable(reason, spent);
    };
    let lost = lost_entries(&checkout, &candidates, &history);
    HistoryReport {
        schema_version: 1,
        resolution: Resolution::Available { selected, history },
        skipped: spent.skipped,
        lost,
        cost: spent.cost,
    }
}

/// The requests and refusals one read has accumulated.
#[derive(Default)]
struct Spent {
    cost: QueryCost,
    skipped: Vec<Disclosure>,
}

impl Spent {
    fn add(&mut self, cost: QueryCost) {
        self.cost.graphql_points += cost.graphql_points;
        self.cost.rest_requests += cost.rest_requests;
        self.cost.requests.extend(cost.requests);
    }

    /// A failed query reports no cost of its own, yet it reached GitHub.
    fn add_failed(&mut self, path: String) {
        self.cost.rest_requests += 1;
        self.cost.requests.push(path);
    }

    fn skip(&mut self, run: &Run, reason: &str) {
        self.skipped.push(Disclosure {
            run_id: run.id.to_string(),
            head_sha: run.head_sha.as_str().to_owned(),
            reason: reason.to_owned(),
        });
    }

    fn list_runs(&mut self, request: &HistoryRequest) -> Result<Vec<Run>, String> {
        let answer = host_query::runs(&RunsRequest {
            repo: request.repository.clone(),
            branch: request.base_ref.clone(),
            workflow: request.workflow,
            event: "push".into(),
            limit: LISTED_RUNS,
        });
        match answer {
            Ok(page) => {
                self.add(page.cost);
                Ok(page.runs)
            }
            Err(error) => {
                self.add_failed(format!(
                    "repos/{}/actions/workflows/{}/runs",
                    request.repository.as_str(),
                    request.workflow.get()
                ));
                Err(format!("could not list base runs: {error}"))
            }
        }
    }

    /// The listed runs local git proves reach the base head, nearest first; the listing's
    /// own newest-first order only breaks ties between runs of one commit.
    fn reachable(&mut self, checkout: &Checkout<'_>, runs: &[Run]) -> Vec<Run> {
        let mut reachable = Vec::new();
        for run in runs {
            let distance = match checkout.relation_to_base(run.head_sha.as_str()) {
                Relation::Ancestor => checkout.distance_to_base(run.head_sha.as_str()),
                Relation::Unrelated => Err("not reachable from the requested base head".into()),
                Relation::Unknown(reason) => Err(format!("ancestry unproven: {reason}")),
            };
            match distance {
                Ok(distance) => reachable.push((distance, run.clone())),
                Err(reason) => self.skip(run, &reason),
            }
        }
        reachable.sort_by_key(|(distance, _)| *distance);
        reachable.into_iter().map(|(_, run)| run).collect()
    }

    fn read_candidate(
        &mut self,
        request: &HistoryRequest,
        checkout: &Checkout<'_>,
        listed: &[Run],
        run: &Run,
    ) -> Result<(RollingHistory, SelectedArtifact), String> {
        let (artifact_id, artifact) = self.find_artifact(request, run)?;
        if artifact.size_bytes > MAX_SAVED_BYTES {
            return Err(format!(
                "artifact {} is {} bytes, over the {MAX_SAVED_BYTES} byte limit",
                artifact.id, artifact.size_bytes
            ));
        }
        if self.cost.rest_requests >= MAX_REST_REQUESTS {
            return Err(
                "not downloaded: this read's request bound was reached, and older artifacts are never walked"
                    .into(),
            );
        }
        let archive = self.download(request, artifact_id)?;
        let history = rolling_file(&archive)?;
        validate_provenance(request, checkout, listed, run, &history)?;
        let selected = SelectedArtifact {
            run_id: run.id.to_string(),
            head_sha: run.head_sha.as_str().to_owned(),
            attempt: run.run_attempt,
            artifact_id: artifact.id,
            entries: history.runs.len(),
        };
        Ok((history, selected))
    }

    /// One attempt can leave an earlier attempt's artifact of the same name listed. Every
    /// artifact is immutable, so the newest live one stands for the run.
    fn find_artifact(
        &mut self,
        request: &HistoryRequest,
        run: &Run,
    ) -> Result<(PositiveId, HostArtifact), String> {
        let run_id = run
            .id
            .to_string()
            .parse::<PositiveId>()
            .map_err(|reason| format!("run id: {reason}"))?;
        let answer = host_query::run_artifacts(&RunArtifactsRequest {
            repo: request.repository.clone(),
            run_id,
        });
        let page = match answer {
            Ok(page) => {
                self.add(page.cost.clone());
                page
            }
            Err(error) => {
                self.add_failed(format!(
                    "repos/{}/actions/runs/{}/artifacts",
                    request.repository.as_str(),
                    run.id
                ));
                return Err(format!("could not list its artifacts: {error}"));
            }
        };
        let named: Vec<_> = page
            .artifacts
            .into_iter()
            .filter(|artifact| artifact.name == request.artifact_name)
            .collect();
        if named.is_empty() {
            return Err(format!(
                "no artifact named {}: its upload failed or never ran",
                request.artifact_name
            ));
        }
        named
            .into_iter()
            .filter(|artifact| !artifact.expired)
            .filter_map(|artifact| Some((artifact.id.parse::<PositiveId>().ok()?, artifact)))
            .max_by_key(|(id, _)| *id)
            .ok_or_else(|| format!("artifact {} has expired", request.artifact_name))
    }

    fn download(
        &mut self,
        request: &HistoryRequest,
        artifact_id: PositiveId,
    ) -> Result<Vec<u8>, String> {
        let directory =
            tempfile::tempdir().map_err(|error| format!("cannot prepare a download: {error}"))?;
        let output = directory.path().join("artifact.zip");
        let answer = host_query::artifact(&ArtifactRequest {
            repo: request.repository.clone(),
            artifact_id,
            output: output.clone(),
        });
        match answer {
            Ok(download) => self.add(download.cost),
            Err(error) => {
                self.add_failed(format!(
                    "repos/{}/actions/artifacts/{}/zip",
                    request.repository.as_str(),
                    artifact_id.get()
                ));
                return Err(format!("could not download its artifact: {error}"));
            }
        }
        fs::read(&output).map_err(|error| format!("cannot read the download: {error}"))
    }
}

/// The one member the base job's artifact is read for, without extracting anything.
fn rolling_file(archive: &[u8]) -> Result<RollingHistory, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|error| format!("artifact is not a readable archive: {error}"))?;
    let member = archive
        .by_name(ROLLING_HISTORY_FILE)
        .map_err(|error| format!("artifact has no {ROLLING_HISTORY_FILE}: {error}"))?;
    let mut bytes = Vec::new();
    member
        .take(MAX_SAVED_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {ROLLING_HISTORY_FILE}: {error}"))?;
    if bytes.len() as u64 > MAX_SAVED_BYTES {
        return Err(format!(
            "{ROLLING_HISTORY_FILE} is over the {MAX_SAVED_BYTES} byte limit"
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("{ROLLING_HISTORY_FILE} is not a rolling history: {error}"))
}

fn validate_provenance(
    request: &HistoryRequest,
    checkout: &Checkout<'_>,
    listed: &[Run],
    run: &Run,
    history: &RollingHistory,
) -> Result<(), String> {
    validate_history(history)
        .map_err(|reason| format!("{ROLLING_HISTORY_FILE} is invalid: {reason}"))?;
    if history.repository != request.repository.as_str() || history.base_ref != request.base_ref {
        return Err(format!(
            "{ROLLING_HISTORY_FILE} names {} on {}, not the requested {} on {}",
            history.repository,
            history.base_ref,
            request.repository.as_str(),
            request.base_ref
        ));
    }
    let newest = history
        .runs
        .last()
        .ok_or_else(|| format!("{ROLLING_HISTORY_FILE} holds no runs"))?;
    if newest.head_sha != run.head_sha.as_str()
        || newest.run_id != run.id.to_string()
        || u64::from(newest.attempt) != run.run_attempt
    {
        return Err(format!(
            "{ROLLING_HISTORY_FILE} ends at run {} attempt {} of {}, not this run's latest attempt {}",
            newest.run_id, newest.attempt, newest.head_sha, run.run_attempt
        ));
    }
    for (position, entry) in history.runs.iter().enumerate() {
        entry
            .head_sha
            .parse::<CommitSha>()
            .map_err(|reason| format!("embedded head_sha: {reason}"))?;
        let host_disagrees = listed.iter().any(|known| {
            known.id.to_string() == entry.run_id
                && (known.head_sha.as_str() != entry.head_sha
                    || u64::from(entry.attempt) > known.run_attempt)
        });
        if host_disagrees {
            return Err(format!(
                "embedded run {} attempt {} of {} contradicts the host's record of that run",
                entry.run_id, entry.attempt, entry.head_sha
            ));
        }
        let next = history.runs.get(position + 1);
        let descendant = next.map_or(run.head_sha.as_str(), |next| next.head_sha.as_str());
        if !matches!(
            checkout.relation(&entry.head_sha, descendant),
            Relation::Ancestor
        ) {
            return Err(format!(
                "embedded commit {} is not an ancestor of {descendant}, so the file is not in ancestry order on this branch",
                entry.head_sha
            ));
        }
    }
    Ok(())
}

/// Reachable base runs the history does not reflect. A commit older than a full file's
/// oldest entry was trimmed by the contract rather than lost.
fn lost_entries(
    checkout: &Checkout<'_>,
    reachable: &[Run],
    history: &RollingHistory,
) -> Vec<Disclosure> {
    let oldest = history
        .runs
        .first()
        .filter(|_| history.runs.len() == MAX_ROLLING_RUNS)
        .map(|run| run.head_sha.as_str());
    reachable
        .iter()
        .filter_map(|run| {
            let head = run.head_sha.as_str();
            let reason = match history.runs.iter().find(|entry| entry.head_sha == head) {
                Some(entry) if u64::from(entry.attempt) >= run.run_attempt => return None,
                Some(entry) => format!(
                    "the history holds attempt {} of this commit; attempt {} did not reach it",
                    entry.attempt, run.run_attempt
                ),
                None => {
                    if oldest.is_some_and(|oldest| {
                        matches!(checkout.relation(head, oldest), Relation::Ancestor)
                    }) {
                        return None;
                    }
                    "this base commit's entry is not in the selected history".to_owned()
                }
            };
            Some(Disclosure {
                run_id: run.id.to_string(),
                head_sha: head.to_owned(),
                reason,
            })
        })
        .collect()
}

fn clear_output(output: &Path) -> Result<(), String> {
    let parent = output_parent(output);
    if !parent.is_dir() {
        return Err(format!(
            "--output directory {} does not exist; create it and retry",
            parent.display()
        ));
    }
    match fs::remove_file(output) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot replace {}: {error}; choose a writable --output file",
            output.display()
        )),
    }
}

fn write_output(history: &RollingHistory, output: &Path) -> Result<(), String> {
    let failed = |error: &dyn std::fmt::Display| {
        format!(
            "cannot write {}: {error}; choose a writable --output file",
            output.display()
        )
    };
    let mut file =
        tempfile::NamedTempFile::new_in(output_parent(output)).map_err(|error| failed(&error))?;
    let bytes = serde_json::to_vec(history).map_err(|error| failed(&error))?;
    file.write_all(&bytes).map_err(|error| failed(&error))?;
    file.persist(output).map_err(|error| failed(&error))?;
    Ok(())
}

fn output_parent(output: &Path) -> &Path {
    output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
