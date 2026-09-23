//! Adoption skill scope and install skew: the embedded `aicontext-adopt`
//! skill documents the single-run monorepo flow (fixed path order, slice
//! writes, per-scope budget with legal exit to `pending`, one commit per
//! subproject, final summary), and `doctor` reports the installed skill
//! against the binary (missing, current, stale, unmanaged) without ever
//! failing on it. HOME is overridden per invocation, and the fixture repo
//! is a temp copy; nothing here touches the network or the real HOME.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
}

fn run_with_home<const N: usize>(dir: &Path, home: &Path, args: [&str; N]) -> (i32, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .output()
        .expect("run aicontext");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(255), text)
}

fn fresh_home(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-adopt-{tag}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    std::fs::create_dir_all(&base).expect("mkdir");
    base
}

fn git<const N: usize>(dir: &Path, args: [&str; N]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .expect("git");
    assert!(st.success(), "git {args:?} failed");
}

fn copy_dir(src: &Path, dst: &Path) {
    for entry in std::fs::read_dir(src).expect("read fixture") {
        let entry = entry.expect("entry");
        let to = dst.join(entry.file_name());
        if entry.file_type().expect("ft").is_dir() {
            std::fs::create_dir_all(&to).expect("mkdir");
            copy_dir(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), &to).expect("copy");
        }
    }
}

fn fixture_repo(name: &str) -> PathBuf {
    let base = fresh_home(name);
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    copy_dir(&manifest.join("fixtures").join(name), &base);
    git(&base, ["init", "-q"]);
    git(&base, ["add", "-A"]);
    git(&base, ["commit", "-qm", "fixture"]);
    base
}

fn skill_dir(home: &Path) -> PathBuf {
    home.join(".pi/agent/skills/aicontext-adopt")
}

fn installed_skill_body() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills/aicontext-adopt/SKILL.md");
    std::fs::read_to_string(&path).expect("embedded skill source")
}

/// The `skill` diagnostic of `doctor --json`, looked up by name.
fn skill_diagnostic(dir: &Path, home: &Path) -> (i32, serde_json::Value) {
    let (code, out) = run_with_home(dir, home, ["doctor", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out)
        .unwrap_or_else(|e| panic!("doctor --json must be JSON ({e}): {out}"));
    let found = v["diagnostics"]
        .as_array()
        .expect("diagnostics must be an array")
        .iter()
        .find(|d| d["name"] == "skill")
        .unwrap_or_else(|| panic!("skill diagnostic must exist: {v:?}"))
        .clone();
    (code, found)
}

/// Any diagnostic of `doctor --json`, by exact name.
fn diagnostic_named(dir: &Path, home: &Path, name: &str) -> (i32, serde_json::Value) {
    let (code, out) = run_with_home(dir, home, ["doctor", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out)
        .unwrap_or_else(|e| panic!("doctor --json must be JSON ({e}): {out}"));
    let found = v["diagnostics"]
        .as_array()
        .expect("diagnostics must be an array")
        .iter()
        .find(|d| d["name"] == name)
        .unwrap_or_else(|| panic!("diagnostic {name} must exist: {v:?}"))
        .clone();
    (code, found)
}

#[test]
fn installed_skill_documents_the_monorepo_scope() {
    let home = fresh_home("scope");
    let dir = fixture_repo("node-single");
    let (c, out) = run_with_home(&dir, &home, ["agent", "install", "pi", "--json"]);
    assert_eq!(c, 0, "install must succeed: {out}");
    let body = std::fs::read_to_string(skill_dir(&home).join("SKILL.md")).expect("skill");
    assert_eq!(
        body,
        installed_skill_body(),
        "installed skill must equal the binary source"
    );
    for required in [
        "Monorepo scope",
        "fixed\npath order",
        "status: adopted",
        "status: pending",
        "one commit per subproject",
        "Summary final",
    ] {
        assert!(body.contains(required), "skill must document {required:?}");
    }
}

#[test]
fn doctor_warns_when_skill_is_missing() {
    let home = fresh_home("missing");
    let dir = fixture_repo("node-single");
    let (c, _) = run_with_home(&dir, &home, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let (code, found) = skill_diagnostic(&dir, &home);
    assert_eq!(code, 0, "a missing skill must never fail doctor");
    assert_eq!(found["status"], "warn");
    assert!(
        found["detail"]
            .as_str()
            .unwrap_or("")
            .contains("not installed"),
        "{found:?}"
    );
    assert!(
        found["remediation"]
            .as_str()
            .unwrap_or("")
            .contains("agent install pi"),
        "{found:?}"
    );
}

#[test]
fn doctor_reports_current_skill_as_ok() {
    let home = fresh_home("current");
    let dir = fixture_repo("node-single");
    let (c, _) = run_with_home(&dir, &home, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let (ic, _) = run_with_home(&dir, &home, ["agent", "install", "pi"]);
    assert_eq!(ic, 0);
    let (code, found) = skill_diagnostic(&dir, &home);
    assert_eq!(code, 0);
    assert_eq!(found["status"], "ok");
    assert!(
        found["detail"]
            .as_str()
            .unwrap_or("")
            .contains("up to date"),
        "{found:?}"
    );
}

#[test]
fn doctor_detects_a_stale_installed_skill() {
    let home = fresh_home("stale");
    let dir = fixture_repo("node-single");
    let (c, _) = run_with_home(&dir, &home, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    // A previous binary's install: different body, older managed version.
    let dir_skill = skill_dir(&home);
    std::fs::create_dir_all(&dir_skill).expect("mkdir");
    std::fs::write(
        dir_skill.join("SKILL.md"),
        "# aicontext-adopt\n\nold body\n",
    )
    .expect("write");
    std::fs::write(
        dir_skill.join(".aicontext-managed.json"),
        "{\"managed_by\":\"aicontext\",\"version\":\"0.0.1\",\"files\":[\"SKILL.md\"]}",
    )
    .expect("write");
    let (code, found) = skill_diagnostic(&dir, &home);
    assert_eq!(code, 0, "skew must never fail doctor");
    assert_eq!(found["status"], "warn");
    let detail = found["detail"].as_str().unwrap_or("");
    assert!(
        detail.contains("0.0.1"),
        "installed version must be named: {detail}"
    );
    assert!(
        found["remediation"]
            .as_str()
            .unwrap_or("")
            .contains("agent install pi"),
        "{found:?}"
    );
}

#[test]
fn doctor_detects_an_unmanaged_skill() {
    let home = fresh_home("unmanaged");
    let dir = fixture_repo("node-single");
    let (c, _) = run_with_home(&dir, &home, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let dir_skill = skill_dir(&home);
    std::fs::create_dir_all(&dir_skill).expect("mkdir");
    std::fs::write(dir_skill.join("SKILL.md"), "# hand-written skill\n").expect("write");
    let (code, found) = skill_diagnostic(&dir, &home);
    assert_eq!(code, 0);
    assert_eq!(found["status"], "warn");
    assert!(
        found["detail"].as_str().unwrap_or("").contains("unmanaged"),
        "{found:?}"
    );
}

#[test]
fn skew_warnings_never_fail_doctor() {
    // Every skew state keeps exit 0 and `healthy: true` (only Fail blocks).
    let dir = fixture_repo("node-single");
    for tag in ["w-missing", "w-stale"] {
        let home = fresh_home(tag);
        let (c, _) = run_with_home(&dir, &home, ["init", "--non-interactive"]);
        assert_eq!(c, 0);
        if tag == "w-stale" {
            let dir_skill = skill_dir(&home);
            std::fs::create_dir_all(&dir_skill).expect("mkdir");
            std::fs::write(dir_skill.join("SKILL.md"), "stale").expect("write");
        }
        let (code, out) = run_with_home(&dir, &home, ["doctor", "--json"]);
        assert_eq!(code, 0, "{tag}: doctor must stay green: {out}");
        let v: serde_json::Value = serde_json::from_str(&out).expect("JSON");
        assert_eq!(v["healthy"], true, "{tag}");
    }
}

#[test]
fn doctor_reports_the_opencode_skill_as_its_own_diagnostic() {
    let home = fresh_home("skill-opencode");
    let dir = fixture_repo("node-single");
    let (c, _) = run_with_home(&dir, &home, ["init", "--non-interactive"]);
    assert_eq!(c, 0);

    // Nothing installed: both agents report a warn with their own remediation.
    let (code, pi) = skill_diagnostic(&dir, &home);
    assert_eq!(code, 0, "a missing skill must never fail doctor");
    assert_eq!(pi["status"], "warn");
    assert!(
        pi["remediation"]
            .as_str()
            .unwrap_or("")
            .contains("agent install pi"),
        "{pi:?}"
    );
    let (code2, oc) = diagnostic_named(&dir, &home, "skill.opencode");
    assert_eq!(code2, 0);
    assert_eq!(oc["status"], "warn");
    assert!(
        oc["detail"]
            .as_str()
            .unwrap_or("")
            .contains("not installed for opencode"),
        "{oc:?}"
    );
    assert!(
        oc["remediation"]
            .as_str()
            .unwrap_or("")
            .contains("agent install opencode"),
        "{oc:?}"
    );

    // Installing for opencode flips only that entry; pi stays a warn.
    let (ic, iout) = run_with_home(&dir, &home, ["agent", "install", "opencode"]);
    assert_eq!(ic, 0, "{iout}");
    let (_, oc2) = diagnostic_named(&dir, &home, "skill.opencode");
    assert_eq!(oc2["status"], "ok", "{oc2:?}");
    assert!(
        oc2["detail"]
            .as_str()
            .unwrap_or("")
            .contains("up to date for opencode"),
        "{oc2:?}"
    );
    let (_, pi2) = skill_diagnostic(&dir, &home);
    assert_eq!(pi2["status"], "warn", "pi must stay independent: {pi2:?}");

    // Still green: every skill state is a warning, never a failure.
    let (dc, dout) = run_with_home(&dir, &home, ["doctor", "--json"]);
    assert_eq!(dc, 0, "doctor must stay green: {dout}");
}
