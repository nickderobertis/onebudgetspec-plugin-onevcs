//! Real producer probe and onevcs, with only crates.io replaced at the curl boundary.
#![cfg(unix)]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn checked(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
struct Fixture {
    temp: tempfile::TempDir,
    repo: PathBuf,
    bin: PathBuf,
    onevcs: PathBuf,
    id: String,
    probe: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("checkout");
        let bin = temp.path().join("bin");
        fs::create_dir_all(repo.join("scripts")).unwrap();
        fs::create_dir(&bin).unwrap();
        let declaration: toml::Table = fs::read_to_string(root().join("release-targets.toml"))
            .unwrap()
            .parse()
            .unwrap();
        let id = declaration["target"][0]["id"].as_str().unwrap().to_owned();
        let probe = PathBuf::from(declaration["probe"].as_str().unwrap());
        for path in [
            PathBuf::from("release-targets.toml"),
            probe.clone(),
            PathBuf::from("scripts/release_probe.py"),
        ] {
            fs::copy(root().join(&path), repo.join(&path)).unwrap();
        }
        checked(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .arg(&repo),
        );
        checked(
            Command::new("git")
                .args(["init", "-q", "--bare"])
                .arg(temp.path().join("origin.git")),
        );
        checked(
            Command::new("git")
                .current_dir(&repo)
                .args(["remote", "add", "origin"])
                .arg(temp.path().join("origin.git")),
        );
        checked(Command::new("git").current_dir(&repo).args(["add", "."]));
        checked(Command::new("git").current_dir(&repo).args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "feat: declare scaffold",
        ]));
        let onevcs = std::env::var_os("ONEVCS_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("onevcs"));
        let fixture = Self {
            temp,
            repo,
            bin,
            onevcs,
            id,
            probe,
        };
        checked(fixture.vcs().arg("register").arg(&fixture.repo));
        fixture
    }
    fn vcs(&self) -> Command {
        let mut command = Command::new(&self.onevcs);
        command
            .env("ONEVCS_HOME", self.temp.path().join("state"))
            .env(
                "PATH",
                format!("{}:{}", self.bin.display(), std::env::var("PATH").unwrap()),
            );
        command
    }
    fn registry(&self, status: &str, body: &str, failure: bool) {
        fs::write(
            self.temp.path().join("response"),
            format!("{body}\n{status}"),
        )
        .unwrap();
        let script = format!(
            "#!/bin/sh\n{}\n",
            if failure {
                "echo 'request failed' >&2; exit 28".to_string()
            } else {
                format!("cat '{}'", self.temp.path().join("response").display())
            }
        );
        let path = self.bin.join("curl");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn ask(&self, id: &str) -> Output {
        Command::new(self.repo.join(&self.probe))
            .arg(id)
            .current_dir(self.temp.path())
            .env(
                "PATH",
                format!("{}:{}", self.bin.display(), std::env::var("PATH").unwrap()),
            )
            .output()
            .unwrap()
    }
    fn latest(&self) -> Output {
        self.vcs()
            .args(["release", "latest"])
            .arg(&self.repo)
            .args(["--target", "crate", "--json"])
            .output()
            .unwrap()
    }
}
#[test]
fn declaration_and_three_answers_round_trip_through_real_onevcs() {
    let f = Fixture::new();
    let output = checked(
        f.vcs()
            .args(["release", "declaration"])
            .arg(&f.repo)
            .arg("--json"),
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["target"][0]["id"], f.id);
    let name = f.id.strip_prefix("crate:").unwrap();
    for (status, body, state, version) in [
        (
            "200",
            format!(r#"{{"crate":{{"id":"{name}","max_stable_version":"1.2.3"}}}}"#),
            "released",
            Some("1.2.3"),
        ),
        ("404", "{}".into(), "no-release", None),
        (
            "200",
            format!(r#"{{"crate":{{"id":"{name}","max_stable_version":null}}}}"#),
            "no-release",
            None,
        ),
    ] {
        f.registry(status, &body, false);
        let direct = f.ask(&f.id);
        assert!(
            direct.status.success(),
            "{}",
            String::from_utf8_lossy(&direct.stderr)
        );
        assert_eq!(
            String::from_utf8(direct.stdout).unwrap().trim(),
            version.unwrap_or("")
        );
        let latest = f.latest();
        assert!(
            latest.status.success(),
            "{}",
            String::from_utf8_lossy(&latest.stderr)
        );
        let latest: serde_json::Value = serde_json::from_slice(&latest.stdout).unwrap();
        assert_eq!(latest["state"], state);
        if let Some(version) = version {
            assert_eq!(latest["version"], version);
        }
    }
    for (body, fail) in [("not JSON", false), ("", true)] {
        f.registry("200", body, fail);
        let direct = f.ask(&f.id);
        assert!(!direct.status.success());
        assert!(direct.stdout.is_empty());
        assert!(!direct.stderr.is_empty());
        let latest = f.latest();
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&latest.stdout),
            String::from_utf8_lossy(&latest.stderr)
        );
        assert!(
            combined.contains("not-answered") || combined.contains("not answered"),
            "{combined}"
        );
        assert!(combined.contains("release probe"), "{combined}");
    }
    let unknown = f.ask("crate:unknown");
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown identifier"));
    let unknown = f
        .vcs()
        .args(["release", "latest"])
        .arg(&f.repo)
        .args(["--target", "unknown"])
        .output()
        .unwrap();
    assert!(!unknown.status.success());
    assert!(!unknown.stderr.is_empty());
}

#[test]
fn malformed_declarations_usage_and_registry_documents_are_refused() {
    let f = Fixture::new();
    let name = f.id.strip_prefix("crate:").unwrap();
    for (status, body) in [
        ("503", "unavailable".to_owned()),
        ("abc", "{}".to_owned()),
        ("200", "{}".to_owned()),
        ("200", "[]".to_owned()),
        (
            "200",
            r#"{"crate":{"id":"another","max_stable_version":"1.0.0"}}"#.to_owned(),
        ),
        ("200", format!(r#"{{"crate":{{"id":"{name}"}}}}"#)),
        (
            "200",
            format!(r#"{{"crate":{{"id":"{name}","max_stable_version":12}}}}"#),
        ),
        (
            "200",
            format!(r#"{{"crate":{{"id":"{name}","max_stable_version":"1.2"}}}}"#),
        ),
        (
            "200",
            format!(r#"{{"crate":{{"id":"{name}","max_stable_version":"1.2.3-rc.1"}}}}"#),
        ),
    ] {
        f.registry(status, &body, false);
        let output = f.ask(&f.id);
        assert!(!output.status.success(), "{status} {body}: {output:?}");
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("release probe:"));
    }
    f.registry(
        "200",
        &format!(r#"{{"crate":{{"id":"{name}","max_stable_version":"1.2.3+build.4"}}}}"#),
        false,
    );
    let output = f.ask(&f.id);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1.2.3+build.4\n");
    for args in [vec![], vec![&f.id, &f.id]] {
        let output = Command::new(f.repo.join(&f.probe))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("expected one declared"));
    }
    let path = f.repo.join("release-targets.toml");
    let declaration = fs::read_to_string(&path).unwrap();
    for replacement in [
        None,
        Some("not [ valid TOML".to_owned()),
        Some(declaration.replace(&f.id, "pypi:test")),
    ] {
        match replacement.as_ref() {
            Some(text) => fs::write(&path, text).unwrap(),
            None => fs::remove_file(&path).unwrap(),
        };
        let id = if replacement
            .as_ref()
            .is_some_and(|s| s.contains("pypi:test"))
        {
            "pypi:test"
        } else {
            &f.id
        };
        let output = f.ask(id);
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
        assert!(output.stdout.is_empty());
        fs::write(&path, &declaration).unwrap();
    }
}
