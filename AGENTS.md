# AGENTS.md

`CLAUDE.md` is a symlink; edit this file only. Keep instructions terse and durable,
put deterministic steps in recipes, and localize subtree rules.

## What this repo is

A general onebudgetspec-to-onevcs tool, independent of GitHub Actions and PR comments: the
reporter protocol over stdin/stdout, and the `history` verb that reads one saved rolling
history. onebudgetspec-core owns the wire contracts and budget semantics; onevcs owns host
access; local git proves ancestry. The action repository consumes the published crate.

## Two standing goals on every task

Prioritize the user's task while improving context (realistic tests and concise
instructions) and maintainability (repeatable setup and strict gates). Propose
material improvements outside the task as follow-ups.

## Stack and composition

- **Product shape:** cli, supplemented with library guidance.
- **Language(s):** rust
<!-- llmlint: ignore-block[agents_md_durable_and_terse] The task and create-repo require the emitted references and composition invocation here so future maintainers can reproduce the baseline and review optional exclusions. -->
- **References composed:** base.md, project-graph.md, shapes/cli.md,
  languages/rust.md, intersections/rust-cli.md, ci.md, llmlint.md, releasing.md,
  tools/onebudgetspec.md; supplemented with shapes/library.md from dero-skills
  create-repo v1.52.0. Composed with `--shape cli --language rust --releasing`.
<!-- llmlint: ignore-end[agents_md_durable_and_terse] -->
- **Projects in the graph:** published crate, binary e2e leaf, repository checks,
  packaged install journey, and workspace tooling. The e2e depends only on the
  binary build; repository contracts have their own tests.
- **Excluded, and why:** asdf/direnv (native toolchain files suffice); downloaded
  archives/installers and npm/PyPI wrappers (cargo is the sole installation
  surface); action.yml (the action has its own repository); live tests (only GitHub's HTTP
  boundary is doubled); measured budget thresholds (no measurements yet).
- Nx is mandatory even for this single deliverable; a graph boundary check keeps
  e2e a leaf. No JS module-boundary plugin is needed for Rust-only sources.
- Linux/macOS run the POSIX development gate; the packaged tool installation is
  also proven on Windows. No MSRV promise yet: use the pinned stable toolchain.

## Command surface

Use `just --list`. The full sweep belongs at release-prep on release PRs; ordinary
PRs and main pushes use the affected tier. The separate install job proves
packaging once per platform. No later release operation re-gates that tree.
Judged lint stays separate from the deterministic gate. Keep the narrow command
allowlist current when adding routine commands.

## Commits, releases, and merging

Squash-only merging, ordinary-change auto-merge allowed, head branches deleted,
all gating checks required; admins may override in an emergency. PR titles follow
Conventional Commits and the PR template supplies the squash body.

Use semantic versioning: pre-1.0 feat/breaking changes bump minor,
fix/perf/refactor/build bump patch; docs/chore/ci/test/style do not release.
After 1.0 breaking changes bump major. Release-plz computes versions/changelogs from conventional history and arms auto-merge on
its release PR, under the release PAT. Merging that PR tags and cuts the GitHub Release;
`release.yml` then publishes the crate idempotently, re-gating nothing. Nobody hand-edits
versions, tags or deploys.

## Invariants and tests

Validate inputs at trust boundaries, keep permissions narrow, and never store
credentials. Tests drive real boundaries, happy and failure paths; only external
registries are doubled.
Coverage is a floor, not a replacement for realistic journeys.

A cost figure belongs in onebudgetspec, not a test threshold; wall-clock tests
wait on events. Time-feature contracts, exact counts and absent operations stay
assertions. Add budgets from measurements, not invented limits.

Scripts should be quiet on success and give the exact failure and next action.
Fix findings at their site; any narrowly justified suppression must explain why.
See [README](README.md) for credential-dependent checks and user-run secret
setup; never sync secrets implicitly.
