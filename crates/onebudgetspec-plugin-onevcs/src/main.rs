use std::{
    io::{self, Read, Write},
    path::PathBuf,
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use onebudgetspec_core::delivery::{DeliveryResult, PublishRequest, Reporter};
use onebudgetspec_plugin_onevcs::{
    history::{self, HistoryRequest, Resolution},
    reporter::{self, OnevcsReporter},
};
use onevcs::host_query::{CommitSha, PositiveId, RepositorySlug};
use serde::Serialize;

/// Deliver onebudgetspec reports and read saved base-branch budget history through onevcs.
/// With no command, answers a PublishRequest on stdin with a DeliveryResult on stdout.
///
/// The default invocation is the reporter protocol `onebudgetspec report --reporter` speaks,
/// one JSON value each way. A delivery failure is `delivered: false` with a reason and
/// exit 0: it never changes a budget verdict.
///
/// GitHub is reached through `gh`, authenticated by GH_TOKEN or GITHUB_TOKEN.
#[derive(Debug, Parser)]
#[command(name = "onebudgetspec-plugin-onevcs", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Write the newest rolling history a base head can prove it descends from.
    ///
    /// Run it inside a full-history checkout. Exit 0: PATH holds schema-conforming
    /// RollingHistory JSON. Exit 1: no eligible history, and PATH does not exist. Either
    /// way, standard output is a JSON report of the selected artifact, every refused or
    /// lost run with its reason, and the REST request cost.
    History(HistoryArgs),
}

#[derive(Debug, clap::Args)]
struct HistoryArgs {
    /// Repository as OWNER/NAME.
    #[arg(long)]
    repo: RepositorySlug,
    /// Exact base branch name.
    #[arg(long)]
    base_ref: String,
    /// Base head commit every saved entry must reach.
    #[arg(long)]
    base_sha: CommitSha,
    /// Numeric workflow id whose push runs saved the history.
    #[arg(long)]
    workflow: PositiveId,
    /// Name of the artifact holding the rolling file.
    #[arg(long)]
    artifact_name: String,
    /// Where to write the RollingHistory JSON.
    #[arg(long)]
    output: PathBuf,
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            return match error.print() {
                Ok(()) => ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2)),
                Err(failure) => write_failed(&failure),
            };
        }
    };
    let (answer, status) = match cli.command {
        None => (write_json(&report_stdin()), ExitCode::SUCCESS),
        Some(Command::History(args)) => run_history(args),
    };
    match answer {
        Ok(()) => status,
        Err(failure) => write_failed(&failure),
    }
}

fn report_stdin() -> DeliveryResult {
    let mut bytes = Vec::new();
    if let Err(error) = io::stdin().lock().read_to_end(&mut bytes) {
        return reporter::undelivered(&format!("could not read the request: {error}"));
    }
    match serde_json::from_slice::<PublishRequest>(&bytes) {
        Ok(request) => OnevcsReporter.publish(&request),
        Err(error) => reporter::undelivered(&format!("malformed PublishRequest: {error}")),
    }
}

fn run_history(args: HistoryArgs) -> (io::Result<()>, ExitCode) {
    let request = HistoryRequest {
        repository: args.repo,
        base_ref: args.base_ref,
        base_sha: args.base_sha,
        workflow: args.workflow,
        artifact_name: args.artifact_name,
        checkout: PathBuf::from("."),
    };
    let report = history::read_to_file(&request, &args.output);
    let answer = write_json(&report);
    if let Resolution::Unavailable { reason } = &report.resolution {
        let diagnostic = writeln!(io::stderr().lock(), "error: history unavailable: {reason}");
        return (answer.and(diagnostic), ExitCode::FAILURE);
    }
    (answer, ExitCode::SUCCESS)
}

fn write_json(value: &impl Serialize) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    writeln!(stdout)
}

fn write_failed(error: &io::Error) -> ExitCode {
    // Reporting is best-effort when the diagnostic stream itself is broken.
    let _ = writeln!(
        io::stderr().lock(),
        "error: could not write output: {error}; restore the output stream and retry."
    );
    ExitCode::FAILURE
}
