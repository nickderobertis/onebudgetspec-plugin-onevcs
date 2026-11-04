//! Structural release proof: authenticated publication is deliberately not executed.
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
