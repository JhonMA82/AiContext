use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn installer_path(name: &str) -> PathBuf {
    repo_root().join("installers").join(name)
}

fn read_script(name: &str) -> String {
    std::fs::read_to_string(installer_path(name)).expect("installer must exist")
}

fn run_sh(name: &str, args: &[&str]) -> (i32, String) {
    let out = Command::new("sh")
        .arg(installer_path(name))
        .args(args)
        .output()
        .expect("run sh script");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(255), text)
}

#[test]
fn all_remote_scripts_exist() {
    for name in [
        "install.sh",
        "update.sh",
        "uninstall.sh",
        "install.ps1",
        "update.ps1",
        "uninstall.ps1",
        "README.md",
    ] {
        assert!(installer_path(name).exists(), "{name} must exist");
    }
}

#[test]
fn shell_scripts_pass_syntax_check() {
    for name in ["install.sh", "update.sh", "uninstall.sh"] {
        let out = Command::new("sh")
            .args(["-n", &installer_path(name).to_string_lossy()])
            .output()
            .expect("sh -n");
        assert!(
            out.status.success(),
            "{name} must pass `sh -n`: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn install_script_uses_official_release_source() {
    let body = read_script("install.sh");
    assert!(body.contains("set -eu"), "must be strict POSIX sh");
    assert!(
        body.contains("JhonMA82/AiContext"),
        "must point at the GitHub repo"
    );
    assert!(
        body.contains("releases"),
        "must point at GitHub Releases"
    );
    assert!(
        body.contains("-installer.sh"),
        "must reuse the official cargo-dist installer"
    );
    assert!(
        body.contains("releases/latest/download"),
        "latest must resolve via the latest/download channel"
    );
    assert!(
        body.contains("releases/download/v"),
        "pinned versions must use releases/download/v<VERSION>"
    );
    assert!(
        body.contains("cargo install"),
        "must document/offer the cargo fallback"
    );
    assert!(body.contains("--version"), "must support --version");
    assert!(body.contains("--help"), "must support --help");
    assert!(!body.contains("sudo"), "must never use sudo");
    assert!(
        body.contains("AICONTEXT_INSTALL_DIR"),
        "must honor the dist install-dir env contract"
    );
    assert!(
        body.contains("dirname"),
        "must pass the parent of the bin dir to the official installer (it appends `bin` itself; avoids `<dir>/bin/bin`)"
    );
}

#[test]
fn update_script_check_is_readonly_and_yes_guarded() {
    let body = read_script("update.sh");
    assert!(body.contains("dirname"), "must share install.sh base-dir contract");
    assert!(body.contains("--check"), "must support --check");
    assert!(body.contains("--yes"), "must support --yes");
    assert!(
        body.contains("refusing to update without --yes"),
        "must refuse mutation without --yes like self update"
    );
    assert!(
        body.contains("self update --check"),
        "--check must delegate to the installed binary when present"
    );
    let (code, out) = run_sh("update.sh", &[]);
    assert_eq!(code, 2, "update without --yes must exit 2: {out}");
    let (code, out) = run_sh("update.sh", &["--help"]);
    assert_eq!(code, 0, "--help must succeed: {out}");
    assert!(out.contains("Usage"), "--help must print usage: {out}");
}

#[test]
fn uninstall_script_only_touches_owned_state() {
    let body = read_script("uninstall.sh");
    assert!(body.contains("--managed"), "must support --managed");
    assert!(body.contains("--yes"), "must support --yes");
    assert!(
        body.contains("refusing to uninstall without --yes"),
        "must refuse without --yes like self uninstall"
    );
    assert!(
        body.contains(".aicontext-managed.json"),
        "skill removal must be manifest-gated"
    );
    assert!(
        body.contains("managed-tools.json"),
        "must consult the ownership registry with --managed"
    );
    assert!(
        body.contains("bin/bin"),
        "must clean legacy nested `<dir>/bin/bin` binaries from early wrappers"
    );
    assert!(
        body.contains("left untouched"),
        "must report foreign files instead of removing them"
    );
    assert!(
        body.contains(".engineering"),
        "must document that .engineering/ is never touched"
    );
    assert!(!body.contains("rm -rf /"), "must never contain rm -rf /");
    assert!(!body.contains("sudo"), "must never use sudo");
    let (code, out) = run_sh("uninstall.sh", &[]);
    assert_eq!(code, 2, "uninstall without --yes must exit 2: {out}");
    assert!(
        out.contains("uninstall.sh --managed --yes"),
        "must print the exact remediation: {out}"
    );
}

#[test]
fn powershell_scripts_mirror_unix_contract() {
    for name in ["install.ps1", "update.ps1"] {
        let body = read_script(name);
        assert!(
            body.contains("Split-Path"),
            "{name} must pass the parent of the bin dir to the official installer"
        );
        assert!(
            body.contains("JhonMA82/AiContext"),
            "{name} must point at the GitHub repo"
        );
        assert!(
            body.contains("releases"),
            "{name} must point at GitHub Releases"
        );
        assert!(
            body.contains("AICONTEXT_INSTALL_DIR"),
            "{name} must honor the install-dir env contract"
        );
    }
    for name in ["install.ps1", "update.ps1", "uninstall.ps1"] {
        let body = read_script(name);
        assert!(
            body.contains("AICONTEXT_INSTALL_DIR"),
            "{name} must honor the install-dir env contract"
        );
    }
    for name in ["uninstall.ps1"] {
        let body = read_script(name);
        assert!(
            body.contains("left alone"),
            "{name} must report foreign files instead of removing them"
        );
    }
    let update = read_script("update.ps1");
    assert!(update.contains("-Check"), "update.ps1 must support -Check");
    assert!(update.contains("-Yes"), "update.ps1 must support -Yes");
    let uninstall = read_script("uninstall.ps1");
    assert!(
        uninstall.contains("-Managed"),
        "uninstall.ps1 must support -Managed"
    );
    assert!(
        uninstall.contains(".aicontext-managed.json"),
        "uninstall.ps1 skill removal must be manifest-gated"
    );
    let install = read_script("install.ps1");
    assert!(
        install.contains("-installer.ps1"),
        "install.ps1 must reuse the official cargo-dist powershell installer"
    );
}
