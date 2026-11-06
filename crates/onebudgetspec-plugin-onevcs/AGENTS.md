Budget semantics and wire types belong to onebudgetspec-core: import them, never restate
them. GitHub is reached only through onevcs's public API and ancestry only through local git;
add no HTTP client. The `history` verb's flags are what the action invokes.

`tests/fixtures/github-wire.json` is the one record of GitHub's answers, and both HTTP doubles
read it. Never hand-edit it: `just record-github-wire` rewrites it, and a test fails when onevcs
sends a query other than the recorded one.
