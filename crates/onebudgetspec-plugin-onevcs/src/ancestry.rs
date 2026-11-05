//! Local git as the ancestry proof: no host request is made here.
use std::{
    path::Path,
    process::{Command, Output},
};

use onevcs::host_query::CommitSha;

/// What local git can say about one commit relative to another.
pub(crate) enum Relation {
    /// The first commit is the second, or one of its ancestors.
    Ancestor,
    /// Both commits are known and the first is not an ancestor of the second.
    Unrelated,
    /// Git could not answer, so nothing is proven.
    Unknown(String),
}

/// A full-history checkout holding the requested base head.
pub(crate) struct Checkout<'a> {
    directory: &'a Path,
    base: &'a CommitSha,
}

impl<'a> Checkout<'a> {
    /// Refuse a checkout that cannot prove ancestry, before any host request is spent.
    pub(crate) fn open(directory: &'a Path, base: &'a CommitSha) -> Result<Self, String> {
        let checkout = Self { directory, base };
        let shallow = checkout
            .git(&["rev-parse", "--is-shallow-repository"])
            .map_err(|reason| format!("{reason}; run this inside the base checkout"))?;
        if String::from_utf8_lossy(&shallow.stdout).trim() != "false" {
            return Err(format!(
                "{} is a shallow clone, so ancestry cannot be proven; check out with full history (fetch-depth: 0)",
                directory.display()
            ));
        }
        checkout
            .git(&[
                "cat-file",
                "-e",
                &format!("{}^{{commit}}", base.as_str()),
            ])
            .map_err(|reason| {
                format!(
                    "base commit {} is not available locally ({reason}); fetch it before reading history",
                    base.as_str()
                )
            })?;
        Ok(checkout)
    }

    /// Whether `commit` is reachable from `descendant`. Both must already be validated
    /// object names, which is why neither can be read as an option.
    pub(crate) fn relation(&self, commit: &str, descendant: &str) -> Relation {
        let answer = Command::new("git")
            .current_dir(self.directory)
            .args(["merge-base", "--is-ancestor", commit, descendant])
            .output();
        match answer {
            Ok(output) => match output.status.code() {
                Some(0) => Relation::Ancestor,
                Some(1) => Relation::Unrelated,
                _ => Relation::Unknown(one_line(&output.stderr)),
            },
            Err(error) => Relation::Unknown(format!("could not run git: {error}")),
        }
    }

    /// Whether `commit` is reachable from the requested base head.
    pub(crate) fn relation_to_base(&self, commit: &str) -> Relation {
        self.relation(commit, self.base.as_str())
    }

    /// Commits between a reachable `commit` and the base head: the ancestry order that
    /// picks the nearest run, whatever its timestamps say.
    pub(crate) fn distance_to_base(&self, commit: &str) -> Result<u64, String> {
        let range = format!("{commit}..{}", self.base.as_str());
        let output = self.git(&["rev-list", "--count", &range])?;
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .map_err(|error| format!("git rev-list answered no count: {error}"))
    }

    fn git(&self, args: &[&str]) -> Result<Output, String> {
        let output = Command::new("git")
            .current_dir(self.directory)
            .args(args)
            .output()
            .map_err(|error| format!("could not run git: {error}"))?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(format!(
                "git {} failed: {}",
                args.join(" "),
                one_line(&output.stderr)
            ))
        }
    }
}

fn one_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
