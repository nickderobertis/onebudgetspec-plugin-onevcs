//! The real `release-plz update`, under this repository's own `release-plz.toml` and the
//! published crate's real manifest, over a controlled git history. The registry is
//! controlled too: the last release is a local checkout of its tag
//! (`--registry-manifest-path`), and crates.io is replaced by a vendored directory of stub
//! crates, so nothing touches the network.
//!
//! Each journey tags a release, then adopts a new version of one dependency of the
//! published crate in whichever manifest declares it, and asserts what release-plz
//! proposes. Needs `release-plz` on PATH (`just bootstrap` installs it).
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use toml::{Table, Value};

const PUBLISHED: &str = "onebudgetspec-plugin-onevcs";
const PUBLISHED_DIR: &str = "crates/onebudgetspec-plugin-onevcs";
const DEPENDENCY_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn table(path: &Path) -> Table {
    fs::read_to_string(path).unwrap().parse().unwrap()
}

fn run(dir: &Path, program: &str, args: &[&str], fixture: &Fixture) -> String {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", &fixture.gitconfig)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .unwrap_or_else(|error| {
            panic!("cannot run {program} ({error}); install it with `just bootstrap` and retry")
        });
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "{program} {args:?} failed:\n{text}"
    );
    text
}

/// Where a dependency of the published crate is declared: its own manifest, or the
/// workspace root it inherits from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Declared {
    Crate,
    Workspace,
}

fn version_of(spec: &Value) -> Option<&str> {
    match spec {
        Value::String(version) => Some(version),
        Value::Table(spec) => spec.get("version").and_then(Value::as_str),
        _ => None,
    }
}

fn set_version(spec: &mut Value, version: &str) {
    match spec {
        Value::Table(spec) => {
            spec.insert("version".into(), version.into());
        }
        _ => *spec = version.into(),
    }
}

fn features_of(spec: &Value) -> Vec<String> {
    spec.get("features")
        .and_then(Value::as_array)
        .map(|features| {
            features
                .iter()
                .map(|feature| feature.as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The lowest version that satisfies a plain requirement such as `4`, `^0.2.1` or `1.0`.
fn lowest_satisfying(requirement: &str) -> String {
    let bare = requirement.trim_start_matches(['^', '=', '~', ' ']);
    let mut parts: Vec<&str> = bare.split('.').collect();
    assert!(
        parts.len() <= 3 && parts.iter().all(|p| p.parse::<u64>().is_ok()),
        "unsupported requirement {requirement:?}; extend lowest_satisfying"
    );
    parts.resize(3, "0");
    parts.join(".")
}

fn semver(version: &str) -> (u64, u64, u64) {
    let parts: Vec<u64> = version.split('.').map(|p| p.parse().unwrap()).collect();
    (parts[0], parts[1], parts[2])
}

/// A copy of this workspace reduced to what release-plz reads: the real root and
/// published-crate manifests, the real release configuration, stub sources, stub
/// unpublished members, and stub registry crates.
struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    registry: PathBuf,
    gitconfig: PathBuf,
    workspace: Table,
    published: Table,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let registry = temp.path().join("released");
        let gitconfig = temp.path().join("gitconfig");
        fs::write(
            &gitconfig,
            "[user]\n\tname = Release Fixture\n\temail = fixture@example.invalid\n\
             [init]\n\tdefaultBranch = main\n[commit]\n\tgpgsign = false\n\
             [tag]\n\tgpgsign = false\n",
        )
        .unwrap();
        let source = root();
        let workspace = table(&source.join("Cargo.toml"));
        let published = table(&source.join(PUBLISHED_DIR).join("Cargo.toml"));
        let fixture = Self {
            _temp: temp,
            repo,
            registry,
            gitconfig,
            workspace,
            published,
        };
        let repo = &fixture.repo;
        fs::create_dir_all(repo.join(PUBLISHED_DIR).join("src")).unwrap();
        fs::create_dir_all(repo.join(".cargo")).unwrap();
        fs::copy(
            source.join("release-plz.toml"),
            repo.join("release-plz.toml"),
        )
        .unwrap();
        fs::write(repo.join("README.md"), "# fixture\n").unwrap();
        fs::write(repo.join("LICENSE"), "MIT\n").unwrap();
        fs::write(repo.join(".gitignore"), "/target\n").unwrap();
        fs::write(
            repo.join(".cargo/config.toml"),
            "[source.crates-io]\nreplace-with = \"fixture\"\n\n\
             [source.fixture]\ndirectory = \"vendor\"\n",
        )
        .unwrap();
        let crate_dir = repo.join(PUBLISHED_DIR);
        fs::write(crate_dir.join("CHANGELOG.md"), "# Changelog\n").unwrap();
        fs::write(crate_dir.join("src/lib.rs"), "").unwrap();
        fs::write(crate_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        for member in fixture.members() {
            if member == PUBLISHED_DIR {
                continue;
            }
            let real = table(&source.join(&member).join("Cargo.toml"));
            let name = real["package"]["name"].as_str().unwrap();
            fs::create_dir_all(repo.join(&member).join("src")).unwrap();
            fs::write(repo.join(&member).join("src/lib.rs"), "").unwrap();
            fs::write(
                repo.join(&member).join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion.workspace = true\n\
                     edition.workspace = true\npublish = false\n"
                ),
            )
            .unwrap();
        }
        run(
            Path::new("."),
            "git",
            &["init", "-q", repo.to_str().unwrap()],
            &fixture,
        );
        fixture
    }

    fn members(&self) -> Vec<String> {
        self.workspace["workspace"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| member.as_str().unwrap().to_owned())
            .collect()
    }

    fn released_version(&self) -> String {
        self.workspace["workspace"]["package"]["version"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Every dependency of the published crate: its name, where its requirement is
    /// declared, and the features it asks for.
    fn dependencies(&self) -> Vec<(String, Declared, Vec<String>)> {
        let mut found = Vec::new();
        for kind in DEPENDENCY_TABLES {
            let Some(deps) = self.published.get(kind).and_then(Value::as_table) else {
                continue;
            };
            for (name, spec) in deps {
                let inherited = spec.get("workspace").and_then(Value::as_bool) == Some(true);
                let mut features = features_of(spec);
                let declared = if inherited {
                    features.extend(features_of(
                        &self.workspace["workspace"]["dependencies"][name.as_str()],
                    ));
                    Declared::Workspace
                } else {
                    Declared::Crate
                };
                found.push((name.clone(), declared, features));
            }
        }
        found
    }

    fn declared(&self, dependency: &str) -> Declared {
        self.dependencies()
            .into_iter()
            .find(|(name, ..)| name == dependency)
            .unwrap_or_else(|| panic!("{PUBLISHED} does not depend on {dependency}"))
            .1
    }

    fn requirement_mut(&mut self, dependency: &str) -> &mut Value {
        match self.declared(dependency) {
            Declared::Workspace => self
                .workspace
                .get_mut("workspace")
                .and_then(|w| w.get_mut("dependencies"))
                .and_then(|d| d.get_mut(dependency))
                .unwrap(),
            Declared::Crate => {
                let kind = DEPENDENCY_TABLES
                    .into_iter()
                    .find(|kind| {
                        self.published
                            .get(*kind)
                            .is_some_and(|deps| deps.get(dependency).is_some())
                    })
                    .unwrap();
                self.published[kind].get_mut(dependency).unwrap()
            }
        }
    }

    fn require(&mut self, dependency: &str, requirement: &str) {
        set_version(self.requirement_mut(dependency), requirement);
    }

    fn requirement(&mut self, dependency: &str) -> String {
        version_of(self.requirement_mut(dependency))
            .unwrap()
            .to_owned()
    }

    /// Write both manifests, vendor a stub of every required dependency version, and
    /// lock against them.
    fn write(&mut self) {
        for (name, _, features) in self.dependencies() {
            let version = lowest_satisfying(&self.requirement(&name));
            let dir = self.repo.join("vendor").join(format!("{name}-{version}"));
            fs::create_dir_all(dir.join("src")).unwrap();
            fs::write(dir.join("src/lib.rs"), "").unwrap();
            let features: String = features
                .iter()
                .map(|feature| format!("{feature} = []\n"))
                .collect();
            fs::write(
                dir.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"{version}\"\n\
                     edition = \"2021\"\n\n[features]\n{features}"
                ),
            )
            .unwrap();
            fs::write(
                dir.join(".cargo-checksum.json"),
                format!("{{\"files\":{{}},\"package\":\"{}\"}}", "0".repeat(64)),
            )
            .unwrap();
        }
        fs::write(
            self.repo.join("Cargo.toml"),
            toml::to_string(&self.workspace).unwrap(),
        )
        .unwrap();
        fs::write(
            self.repo.join(PUBLISHED_DIR).join("Cargo.toml"),
            toml::to_string(&self.published).unwrap(),
        )
        .unwrap();
        run(
            &self.repo,
            "cargo",
            &["generate-lockfile", "--offline", "-q"],
            self,
        );
    }

    fn commit(&self, message: &str) {
        run(&self.repo, "git", &["add", "-A"], self);
        run(&self.repo, "git", &["commit", "-q", "-m", message], self);
    }

    /// Tag the current history as the last release and keep a checkout of that tag as
    /// what the registry serves.
    fn release(&self) {
        let tag = format!("v{}", self.released_version());
        run(&self.repo, "git", &["tag", &tag], self);
        let (repo, registry) = (self.repo.to_str().unwrap(), self.registry.to_str().unwrap());
        run(
            Path::new("."),
            "git",
            &["clone", "-q", repo, registry],
            self,
        );
        run(&self.registry, "git", &["checkout", "-q", &tag], self);
    }

    /// Paths the last commit changed.
    fn changed(&self) -> Vec<String> {
        run(
            &self.repo,
            "git",
            &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"],
            self,
        )
        .lines()
        .map(str::to_owned)
        .collect()
    }

    /// Run the real `release-plz update` and return the version it proposes for the
    /// published crate, or `None` when it proposes no release of it.
    ///
    /// A proposal is the crate's new changelog entry: release-plz may bump the shared
    /// `[workspace.package].version` even for a package it then declines to release (its
    /// `release_commits` filter runs after that bump), so the version field alone is not
    /// evidence. When an entry is written, the manifest must carry the same version.
    fn proposed_version(&self) -> Option<String> {
        let released = self.registry.join("Cargo.toml");
        let log = run(
            &self.repo,
            "release-plz",
            &[
                "update",
                "--config",
                "release-plz.toml",
                "--registry-manifest-path",
                released.to_str().unwrap(),
            ],
            self,
        );
        let diff = run(&self.repo, "git", &["diff"], self);
        eprintln!("release-plz update over the fixture:\n{log}\n{diff}");
        let changelog =
            fs::read_to_string(self.repo.join(PUBLISHED_DIR).join("CHANGELOG.md")).unwrap();
        let entry = changelog.lines().find_map(|line| {
            let rest = line.strip_prefix("## [")?;
            let (version, _) = rest.split_once(']')?;
            (version != "Unreleased").then(|| version.to_owned())
        })?;
        let manifest = table(&self.repo.join(PUBLISHED_DIR).join("Cargo.toml"));
        let version = match manifest["package"].get("version") {
            Some(Value::String(version)) => version.clone(),
            _ => table(&self.repo.join("Cargo.toml"))["workspace"]["package"]["version"]
                .as_str()
                .unwrap()
                .to_owned(),
        };
        assert_eq!(
            entry, version,
            "the changelog entry and the manifest disagree"
        );
        Some(version)
    }
}

/// Adopting a breaking `onebudgetspec-core` (0.1 to 0.2), whose types this crate's public
/// API takes and returns, is proposed as at least the next minor version of this 0.x
/// crate, never a patch, and never skipped.
#[test]
fn a_breaking_public_dependency_adoption_releases_at_least_a_minor_version() {
    let mut fixture = Fixture::new();
    fixture.require("onebudgetspec-core", "0.1.6");
    fixture.write();
    fixture.commit("fix: the released tree");
    fixture.release();

    fixture.require("onebudgetspec-core", "0.2.1");
    fixture.write();
    fixture.commit("feat(deps)!: adopt the onebudgetspec-core that folds earlier heads' flags");

    let released = semver(&fixture.released_version());
    assert_eq!(released.0, 0, "this journey's bump rule is for a 0.x crate");
    let proposed = fixture
        .proposed_version()
        .expect("adopting a breaking onebudgetspec-core must propose a release");
    let proposed = semver(&proposed);
    assert!(
        proposed >= (0, released.1 + 1, 0),
        "adopting a breaking onebudgetspec-core must propose at least 0.{}.0, \
         not {proposed:?}: its types are in {PUBLISHED}'s public API",
        released.1 + 1
    );
}

/// A version change to a dependency the published crate still inherits from the
/// workspace changes only the root manifest, yet changes the packaged crate, so it is
/// proposed as a new version.
#[test]
fn an_inherited_dependency_version_change_alone_releases_a_new_version() {
    let mut fixture = Fixture::new();
    let (inherited, ..) = fixture
        .dependencies()
        .into_iter()
        .find(|(_, declared, _)| *declared == Declared::Workspace)
        .expect("the published crate inherits no dependency; this journey no longer applies");
    let requirement = fixture.requirement(&inherited);
    let current = semver(&lowest_satisfying(&requirement));
    fixture.write();
    fixture.commit("fix: the released tree");
    fixture.release();

    let next = format!("{}.{}.{}", current.0 + 1, current.1, current.2);
    fixture.require(&inherited, &next);
    fixture.write();
    fixture.commit(&format!("fix(deps): adopt {inherited} {next}"));
    assert!(
        fixture
            .changed()
            .iter()
            .all(|path| !path.starts_with(&format!("{PUBLISHED_DIR}/"))),
        "the adoption must not touch {PUBLISHED_DIR}: {:?}",
        fixture.changed()
    );

    let released = semver(&fixture.released_version());
    let proposed = fixture.proposed_version().unwrap_or_else(|| {
        panic!("a new {inherited} requirement changes {PUBLISHED}'s packaged manifest, so a release must be proposed")
    });
    let proposed = semver(&proposed);
    assert!(
        proposed > released,
        "a new {inherited} requirement changes {PUBLISHED}'s packaged manifest, \
         so a release past {released:?} must be proposed, not {proposed:?}"
    );
}
