//! Registry publication is irreversible and requires production credentials.
//! Structural wiring assertions and package dry runs cover publication; the cheap
//! authorization shell steps are executed below.
use serde_norway::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn read(p: &str) -> String {
    fs::read_to_string(root().join(p)).unwrap()
}
fn toml(p: &str) -> toml::Table {
    read(p).parse().unwrap()
}
#[test]
fn release_declaration_and_workflow_agree() {
    let declaration = toml("release-targets.toml");
    assert_eq!(declaration["schema_version"].as_integer(), Some(2));
    let targets = declaration["target"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    let target = &targets[0];
    assert_eq!(target["name"].as_str(), Some("crate"));
    let manifest = toml(target["manifest"].as_str().unwrap());
    let name = manifest["package"]["name"].as_str().unwrap();
    assert_eq!(target["id"].as_str().unwrap(), format!("crate:{name}"));
    let config = toml("release-plz.toml");
    let publish: Vec<_> = config["package"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["publish"].as_bool() == Some(true))
        .collect();
    assert_eq!(publish.len(), 1);
    assert_eq!(publish[0]["name"].as_str().unwrap(), name);
    let workflow: Value =
        serde_norway::from_str(&read(".github/workflows/release-plz.yml")).unwrap();
    assert_eq!(
        workflow["on"]["push"]["branches"].as_sequence().unwrap(),
        &[Value::String("main".into())]
    );
    let jobs = workflow["jobs"].as_mapping().unwrap();
    for (job, command) in [("release", "release"), ("release-pr", "release-pr")] {
        let steps = jobs[Value::String(job.into())]["steps"]
            .as_sequence()
            .unwrap();
        let step = steps
            .iter()
            .find(|s| {
                s["run"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with(&format!("release-plz {command} "))
            })
            .unwrap();
        assert!(
            step["run"]
                .as_str()
                .unwrap()
                .contains("--config release-plz.toml")
        );
        assert_eq!(
            step["env"]["GITHUB_TOKEN"].as_str(),
            Some("${{ secrets.RELEASE_PLZ_TOKEN }}")
        );
        if job == "release" {
            assert_eq!(
                step["env"]["CARGO_REGISTRY_TOKEN"].as_str(),
                Some("${{ secrets.CARGO_REGISTRY_TOKEN }}")
            );
            assert_eq!(
                step["if"].as_str(),
                Some("steps.release-pr.outputs.merged == 'true'")
            );
        }
    }
    let text = read(".github/workflows/release-plz.yml");
    assert!(!text.contains("gh pr merge"));
    assert!(!text.contains("workflow_dispatch"));
    assert!(
        read("AGENTS.md")
            .to_lowercase()
            .contains("the release pr is a human checkpoint")
    );
    assert!(!root().join("action.yml").exists());
}
#[test]
fn workflow_secret_names_are_exactly_the_manifest_mappings() {
    let manifest: serde_json::Value = serde_json::from_str(&read("gh-secrets.json")).unwrap();
    assert_eq!(
        manifest["source"],
        serde_json::json!({"type":"bitwarden","default_field":"notes"})
    );
    assert_eq!(
        manifest["destinations"],
        serde_json::json!([{"type":"github","repository":"nickderobertis/onebudgetspec-plugin-onevcs"}])
    );
    let expected = [
        ("RELEASE_PLZ_TOKEN", "GH_TOKEN"),
        ("CARGO_REGISTRY_TOKEN", "CARGO_REGISTRY_TOKEN"),
        ("OPENAI_API_KEY", "OPENAI_API_KEY"),
        ("CLAUDE_CODE_OAUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"),
    ];
    let secrets = manifest["secrets"].as_array().unwrap();
    assert_eq!(secrets.len(), expected.len());
    for (name, item) in expected {
        assert_eq!(
            secrets.iter().find(|s| s["name"] == name).unwrap(),
            &serde_json::json!({"name":name,"item":item})
        );
    }
    let mut used = std::collections::BTreeSet::new();
    for entry in fs::read_dir(root().join(".github/workflows")).unwrap() {
        let text = fs::read_to_string(entry.unwrap().path()).unwrap();
        for rest in text.split("secrets.").skip(1) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            assert!(
                name == "GITHUB_TOKEN" || expected.iter().any(|(s, _)| *s == name),
                "undeclared secret {name}"
            );
            used.insert(name);
        }
    }
    for (name, _) in expected {
        assert!(used.contains(name), "unused mapping {name}");
    }
}

#[test]
fn install_job_provisions_the_graph_before_installing_the_package() {
    let workflow: Value = serde_norway::from_str(&read(".github/workflows/ci.yml")).unwrap();
    let steps = workflow["jobs"]["install"]["steps"].as_sequence().unwrap();
    let commands: Vec<_> = steps
        .iter()
        .filter_map(|step| step["run"].as_str())
        .collect();
    let locked = commands
        .iter()
        .position(|command| *command == "just dependencies")
        .expect("install job needs Nx's locked dependencies");
    let install = commands
        .iter()
        .position(|command| *command == "just install-smoke")
        .unwrap();
    assert!(locked < install);
    assert!(steps.iter().any(|step| {
        step["uses"]
            .as_str()
            .unwrap_or_default()
            .starts_with("oven-sh/setup-bun@")
    }));
}

#[cfg(unix)]
#[test]
fn actual_release_authorization_handles_ordinary_release_and_missing_credentials() {
    use std::{os::unix::fs::PermissionsExt, process::Command};
    let workflow: Value =
        serde_norway::from_str(&read(".github/workflows/release-plz.yml")).unwrap();
    let steps = workflow["jobs"]["release"]["steps"].as_sequence().unwrap();
    let authorize = steps
        .iter()
        .find(|s| s["id"].as_str() == Some("release-pr"))
        .unwrap()["run"]
        .as_str()
        .unwrap();
    assert!(authorize.contains(".merged_at != null"));
    assert!(authorize.contains(".head.ref"));
    let ci: Value = serde_norway::from_str(&read(".github/workflows/ci.yml")).unwrap();
    let prefix = ci["jobs"]["gate"]["if"]
        .as_str()
        .unwrap()
        .split('\'')
        .nth(1)
        .unwrap();
    assert_eq!(
        ci["jobs"]["check-all"]["if"]
            .as_str()
            .unwrap()
            .split('\'')
            .nth(1),
        Some(prefix)
    );
    assert!(authorize.contains(&format!("{prefix}*)")));
    let temp = tempfile::tempdir().unwrap();
    let gh = temp.path().join("gh");
    std::fs::write(&gh, "#!/bin/sh\nprintf '%s\\n' \"$SERVED_BRANCH\"\n").unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    for (branch, expected) in [
        (format!("{prefix}123"), "merged=true"),
        ("ordinary".into(), "merged=false"),
        (String::new(), "merged=false"),
    ] {
        let output_path = temp.path().join("output");
        std::fs::write(&output_path, "").unwrap();
        let output = Command::new("bash")
            .args(["-euo", "pipefail", "-c", authorize])
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    temp.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("SERVED_BRANCH", branch)
            .env("GITHUB_OUTPUT", &output_path)
            .env("GITHUB_REPOSITORY", "fixture/repository")
            .env("GITHUB_SHA", "fixture-commit")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            std::fs::read_to_string(output_path).unwrap().trim(),
            expected
        );
    }
    let guard = steps[0]["run"].as_str().unwrap();
    for present in [false, true] {
        let output = Command::new("bash")
            .args(["-euo", "pipefail", "-c", guard])
            .env(
                "RELEASE_PLZ_TOKEN",
                if present { "test-credential" } else { "" },
            )
            .env(
                "CARGO_REGISTRY_TOKEN",
                if present { "test-credential" } else { "" },
            )
            .output()
            .unwrap();
        assert_eq!(output.status.success(), present);
        if !present {
            assert!(String::from_utf8_lossy(&output.stderr).contains("RELEASE_PLZ_TOKEN"));
        }
    }
}

#[test]
fn cargo_projects_coverage_and_contract_inputs_agree() {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1", "--locked"])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(output.status.success());
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let workspace: serde_json::Value = serde_json::from_str(&read("project.json")).unwrap();
    let measured = workspace["targets"]["coverage"]["dependsOn"][0]["projects"]
        .as_array()
        .unwrap();
    for package in metadata["packages"].as_array().unwrap() {
        let manifest = Path::new(package["manifest_path"].as_str().unwrap());
        let project: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(manifest.parent().unwrap().join("project.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(project["name"], package["name"]);
        assert!(measured.contains(&project["name"]));
        assert!(
            project["targets"]["test"]["options"]["command"]
                .as_str()
                .unwrap()
                .starts_with("bash tools/coverage.sh ")
        );
        assert_eq!(
            project["targets"]["test"]["outputs"][0],
            "{workspaceRoot}/target/llvm-cov-target"
        );
    }
    let project: serde_json::Value =
        serde_json::from_str(&read("crates/repo-checks/project.json")).unwrap();
    assert!(
        project["implicitDependencies"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("workspace"))
    );
    let inputs = project["targets"]["test"]["inputs"].as_array().unwrap();
    for input in [
        "{workspaceRoot}/.github/workflows/**/*",
        "{workspaceRoot}/scripts/**/*",
        "{workspaceRoot}/release*.toml",
    ] {
        assert!(inputs.contains(&serde_json::json!(input)));
    }
    let install: serde_json::Value =
        serde_json::from_str(&read("tools/install/project.json")).unwrap();
    assert_eq!(
        install["implicitDependencies"],
        serde_json::json!(["onebudgetspec-plugin-onevcs"])
    );
    let node_script = root().join("tools/check-boundaries.mjs");
    let valid = std::process::Command::new("node")
        .arg(&node_script)
        .current_dir(root())
        .output()
        .unwrap();
    assert!(valid.status.success(), "{valid:?}");
    let fixture = tempfile::tempdir().unwrap();
    let e2e = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"].as_str().unwrap().ends_with("-e2e"))
        .unwrap()["name"]
        .clone();
    for package in metadata["packages"].as_array().unwrap() {
        let manifest = Path::new(package["manifest_path"].as_str().unwrap());
        let mut project: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(manifest.parent().unwrap().join("project.json")).unwrap(),
        )
        .unwrap();
        if project["tags"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("type:app"))
        {
            project["implicitDependencies"] = serde_json::json!([e2e]);
        }
        let path = fixture
            .path()
            .join("crates")
            .join(package["name"].as_str().unwrap());
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("project.json"), project.to_string()).unwrap();
    }
    let refused = std::process::Command::new("node")
        .arg(node_script)
        .current_dir(fixture.path())
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("must remain leaves"));
}
