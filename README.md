# onebudgetspec-plugin-onevcs

A general onebudgetspec-to-onevcs tool, independent of GitHub Actions: it delivers
onebudgetspec reports to one standing change-request comment and reads saved base-branch
results back. It needs no Actions variables, onevcs session or registry; GitHub is reached
through `gh`, authenticated by `GH_TOKEN` or `GITHUB_TOKEN`. Comparison and rendering stay in
[onebudgetspec-core](https://crates.io/crates/onebudgetspec-core), whose types this imports.

## Installation

Once the first crate has been published, install with stable Rust and Cargo:

```sh
cargo install onebudgetspec-plugin-onevcs --locked
onebudgetspec-plugin-onevcs --help
onebudgetspec-plugin-onevcs --version
```

Supported tool platforms: Linux, macOS and Windows. Until that first release,
CI packages the crate, installs from the extracted packaged directory with
`cargo install --locked --root <empty-root> --path target/package/onebudgetspec-plugin-onevcs-<version>`,
and repeats the real binary tests from outside the checkout. `just install-smoke`
runs that journey locally; it uploads nothing. There is no downloaded-archive
installer, npm wrapper, or PyPI distribution.

## Reporting

With no arguments the executable is the reporter `onebudgetspec report` runs:

```sh
onebudgetspec report --input comparison.json --reporter onebudgetspec-plugin-onevcs \
  --destination https://github.com/OWNER/NAME/pull/1 --key budget-report
```

It reads one `PublishRequest` from stdin, reads the keyed comment the authenticated author
owns, merges it through core's bounded per-head history, upserts it against the request's
`head_sha`, and writes one `DeliveryResult` to stdout. There is one comment and no description
edit or other history store. A stale head, refused permission, failed post or malformed
request is `delivered: false` with a reason and exit 0; the caller's budget verdict is untouched.
Callers serialize writers per destination and key, because the host has no atomic create.

## History

```sh
onebudgetspec-plugin-onevcs history --repo OWNER/NAME --base-ref REF --base-sha SHA \
  --workflow ID --artifact-name NAME --output PATH
```

Run it inside a full-history checkout. It lists one page of completed push runs of that
workflow on that exact branch, keeps those local git proves reach `--base-sha`, and reads the
nearest by ancestry, never by timestamp. The run's artifact `NAME` must hold
`rolling-history.json` at its root: a core `RollingHistory`, oldest commit first, whose last
entry is that run's latest attempt at its head and whose every embedded commit is an ancestor
of the next. A file failing any of that is refused whole and the next run is tried, within five
REST requests for the whole lookup (three normally); the first valid file is the only one used, and
files are never merged. The limit is checked between onevcs calls, so two gaps remain: one artifact
listing of several pages can pass it before returning, and a failure on a later page reports no
cost, counting that listing as one request. A base run's artifacts normally fit one page.

| Exit | `PATH` | Meaning |
| --- | --- | --- |
| 0 | the `RollingHistory` JSON | a proven history |
| 1 | absent, even if it existed | no eligible history: an unavailable baseline, or a fresh base history; the reason is on stderr |
| 2 | untouched | invalid arguments |

Stdout is JSON on exit 0 and 1: `status`, `selected` or `reason`, `skipped` (refused runs),
`lost` (reachable base commits the history does not hold, for example after a failed upload or
a concurrent push) and `cost`, the onevcs `QueryCost` for job telemetry. Appending a run, trimming
to 20 commits and uploading belong to the caller's base job.

No arguments read stdin; `-h` and `--help` show help; `-V` and `--version` show the package
version. An output I/O failure exits 1. Unsupported arguments print an error and suggested action
to stderr, exit 2, and perform no work.

## Development

Install Rust (rustup), Node, bun, just, uv, Python 3.11+, curl and actionlint.
The development scripts are POSIX (Linux/macOS); the installed binary is portable.
`rust-toolchain.toml` pins stable Rust. Then run:

```sh
just bootstrap
just check
just test-e2e
```

`just --list` lists scoped commands. Nx owns the project/task graph; Cargo owns
one workspace lockfile. The default check uses the explicit NX_BASE (origin/main
locally); `just check all` sweeps every project on a release PR. The aggregate
coverage gate requires 95% lines; supply-chain checks run separately on Linux.
`just upgrade` updates bun/Cargo dependencies and checks the result. Budget tooling
is wired from the start, but there are no invented measurement thresholds.

The journeys also need python3, git and the core CLI that package.json pins (`just dependencies`).
Repository tests need the onevcs version pinned in `tools/onevcs.version`;
bootstrap installs it when absent or mismatched.
The declared release probe needs Python 3.11+ and curl; it reads public crates.io
without credentials. Tests stub only that external registry and run real onevcs
with isolated temporary host state, never modifying the host registry.

The plugin crate's journeys run real `gh` (preinstalled on CI's runners) against a local
server, offline. `tests/fixtures/github-wire.json` holds GitHub's answers; `just record-github-wire`
re-records it read-only from public comments and needs `gh auth`. Run it when the query test
fails after a onevcs upgrade, or to check for GitHub drift, then review the fixture's diff.

## Releases and credentials

Semantic versions and changelogs are computed by release-plz from Conventional
Commits. The release PR has auto-merge armed and lands once its checks pass. Landing it tags
and cuts the GitHub Release, which fires `release.yml` to publish the crate; a re-run skips a
version already live. Ordinary pushes only propose a release PR.

`gh-secrets.json` contains names/mappings only. From this clone, the user may run:

```sh
gh-secrets sync
```

This task does not read, write or sync any secret. Bitwarden items use their notes
field. RELEASE_PLZ_TOKEN maps to GH_TOKEN: a fine-grained GitHub PAT restricted to
this repository with Contents and Pull requests read/write (and Metadata read),
or a classic PAT with public_repo for this public repository. Secret sync itself
needs repository Secrets write permission (classic repo scope also works), plus
Bitwarden read access to the named items. CARGO_REGISTRY_TOKEN needs crates.io
publish permission for this crate (initial publication requires permission to
create it). OPENAI_API_KEY needs model API access; CLAUDE_CODE_OAUTH_TOKEN needs
an active Claude Code subscription credential.

The required `llmlint` CI check awaits provider credentials (OPENAI_API_KEY or
CLAUDE_CODE_OAUTH_TOKEN); validation runs before the model call. Release PR
orchestration awaits RELEASE_PLZ_TOKEN; `release.yml` also awaits
CARGO_REGISTRY_TOKEN. Actions' built-in GITHUB_TOKEN is used only for ordinary
repository operations. A PAT is required for release orchestration so bot-created
PRs/tags can trigger downstream workflows. npm/PyPI credentials are not needed.
