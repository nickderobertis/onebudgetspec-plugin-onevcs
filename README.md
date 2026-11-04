# onebudgetspec-plugin-onevcs

A general onebudgetspec-to-onevcs tool, independent of GitHub Actions and PR
comments. This repository currently supplies an installable Rust scaffold only.
There is no plugin protocol, reporting behavior, onevcs integration, or operational
library API yet. Help and version are the entire command surface.

The future action repository will install and run onebudgetspec with this plugin,
while also supporting direct tool installation. The main plan will choose the
plugin protocol and the action's runtime, inputs and outputs. This scaffold makes
none of those choices and includes no executable action.yml.

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

No arguments, `-h` and `--help` show placeholder help; `-V` and `--version`
show the package version. Exit 0 means success; an output I/O failure exits 1. Unsupported arguments print an
error and suggested action to stderr, exit 2, and perform no work.

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

Repository tests need the onevcs version pinned in `tools/onevcs.version`;
bootstrap installs it when absent or mismatched.
The declared release probe needs Python 3.11+ and curl; it reads public crates.io
without credentials. Tests stub only that external registry and run real onevcs
with isolated temporary host state, never modifying the host registry.

## Releases and credentials

Semantic versions and changelogs are computed by release-plz from Conventional
Commits. A release PR requires human approval; it is never auto-merged. Merging
that PR later authorizes the idempotent crates.io publish/tag/release flow.
Ordinary pushes only propose a release PR. No release has been performed for
this scaffold. The declared release target is the initial crate only; additional
plugin distribution awaits the main plan's interface decision.

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
orchestration awaits RELEASE_PLZ_TOKEN; publishing also awaits
CARGO_REGISTRY_TOKEN. Actions' built-in GITHUB_TOKEN is used only for ordinary
repository operations. A PAT is required for release orchestration so bot-created
PRs/tags can trigger downstream workflows. npm/PyPI credentials are not needed.
