//! Drive the actual startup scripts; external package installation/harness checks are doubled.
#![cfg(unix)]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};
struct Host {
    temp: tempfile::TempDir,
    repo: PathBuf,
    bin: PathBuf,
}
impl Host {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("checkout with spaces");
        let bin = temp.path().join("bin");
        fs::create_dir_all(repo.join("scripts")).unwrap();
        fs::create_dir_all(temp.path().join("home")).unwrap();
        fs::create_dir(&bin).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
        for name in ["session-setup.sh", "setup-llmlint.sh"] {
            fs::copy(source.join(name), repo.join("scripts").join(name)).unwrap();
        }
        for tool in ["bash", "dirname", "mkdir", "chmod"] {
            std::os::unix::fs::symlink(format!("/bin/{tool}"), bin.join(tool)).unwrap();
        }
        Self { temp, repo, bin }
    }
    fn program(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn installer(&self) {
        self.program("uv",r##"#!/bin/bash
printf '%s\n' "$*" >> "$INSTALL_LOG"
[ "${INSTALL_FAIL:-}" != yes ] || { echo 'registry unavailable' >&2; exit 9; }
mkdir -p "$HOME/.local/bin"
case "$*" in
 *rust-just*) printf '#!/bin/bash\nprintf "just test\\n"\n' > "$HOME/.local/bin/just"; chmod +x "$HOME/.local/bin/just" ;;
 *llmlint-cli*)
 printf '#!/bin/bash\nif [ "$1" = doctor ]; then printf "doctor\\n" >> "$DOCTOR_LOG"; exit "${DOCTOR_EXIT:-0}"; fi\nprintf "llmlint test\\n"\n' > "$HOME/.local/bin/llmlint"
 chmod +x "$HOME/.local/bin/llmlint" ;;
esac
"##);
    }
    fn run(&self, script: &str, vars: &[(&str, &str)]) -> Output {
        let mut c = Command::new("/bin/bash");
        c.arg(self.repo.join("scripts").join(script))
            .env_clear()
            .env("HOME", self.temp.path().join("home"))
            .env("PATH", &self.bin)
            .env("INSTALL_LOG", self.temp.path().join("installs"))
            .env("DOCTOR_LOG", self.temp.path().join("doctor"))
            .env("CLAUDE_ENV_FILE", self.temp.path().join("session.env"));
        for (key, value) in vars {
            c.env(key, value);
        }
        c.output().unwrap()
    }
    fn read(&self, file: &str) -> String {
        fs::read_to_string(self.temp.path().join(file)).unwrap_or_default()
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn startup_and_judged_install_cover_install_skip_failure_and_session_recovery() {
    let h = Host::new();
    h.installer();
    let result = h.run("session-setup.sh", &[]);
    success(&result);
    assert!(h.read("installs").contains("rust-just>="));
    assert!(h.read("installs").contains("llmlint-cli>="));
    assert_eq!(h.read("doctor"), "doctor\n");
    assert!(h.read("session.env").contains("export PATH="));
    let h = Host::new();
    h.installer();
    h.program("just", "#!/bin/bash\necho 'just existing'\n");
    success(&h.run("session-setup.sh", &[]));
    assert!(!h.read("installs").contains("rust-just"));
    assert!(h.read("installs").contains("llmlint-cli"));
    let h = Host::new();
    h.installer();
    success(&h.run("session-setup.sh", &[("CI", "true")]));
    assert!(h.read("installs").is_empty());
    assert!(h.read("session.env").is_empty());
    for script in ["session-setup.sh", "setup-llmlint.sh"] {
        let h = Host::new();
        let missing = h.run(script, &[]);
        success(&missing);
        assert!(String::from_utf8_lossy(&missing.stderr).contains("uv not found"));
        h.installer();
        let failed = h.run(script, &[("INSTALL_FAIL", "yes")]);
        success(&failed);
        assert!(String::from_utf8_lossy(&failed.stderr).contains("install failed"));
        let recovered = h.run(script, &[("DOCTOR_EXIT", "8")]);
        success(&recovered);
        assert!(String::from_utf8_lossy(&recovered.stderr).contains("doctor reported an issue"));
        assert!(
            h.read("session.env").contains("export PATH="),
            "{script} must persist the tool path"
        );
    }
    let h = Host::new();
    h.installer();
    success(&h.run("setup-llmlint.sh", &[]));
    assert!(
        h.read("session.env").contains("export PATH="),
        "standalone setup must persist the original PATH delta"
    );
}
