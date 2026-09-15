use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
}

fn run<const N: usize>(dir: &Path, args: [&str; N]) -> (i32, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run aicontext");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(255), text)
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
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-doctor-{name}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    std::fs::create_dir_all(&base).expect("mkdir");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    copy_dir(&manifest.join("fixtures").join(name), &base);
    git(&base, ["init", "-q"]);
    git(&base, ["add", "-A"]);
    git(&base, ["commit", "-qm", "fixture"]);
    base
}

#[test]
fn doctor_healthy_json_contract() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let (cd, out) = run(&dir, ["doctor", "--json"]);
    assert_eq!(cd, 0, "doctor must pass on healthy fixture: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("doctor --json must be JSON");
    assert_eq!(v["schema"], "aicontext/doctor/v1");
    assert_eq!(v["healthy"], true);
    let names: Vec<&str> = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["name"].as_str())
        .collect();
    for expected in ["cli", "git", "config", "state markers", "ast-grep"] {
        assert!(names.contains(&expected), "missing diagnostic {expected}");
    }
}

#[test]
fn doctor_flags_uninitialized_with_remediation() {
    let dir = fixture_repo("rust-single");
    // No init: config diagnostic must fail and point at init.
    let (c, out) = run(&dir, ["doctor", "--json"]);
    assert_eq!(c, 1, "doctor must fail when repo is not initialized");
    let v: serde_json::Value = serde_json::from_str(&out).expect("doctor --json must be JSON");
    assert_eq!(v["healthy"], false);
    let config = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "config")
        .expect("doctor must include a config diagnostic");
    assert_eq!(config["status"], "fail");
    assert!(
        config["remediation"].as_str().unwrap().contains("init"),
        "remediation must point at init"
    );
}

#[test]
fn doctor_warns_on_broken_patterns_refs() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let path = dir.join(".engineering/PATTERNS.md");
    std::fs::write(
        &path,
        "# Patterns\n\nReference:\n- src/does-not-exist-here\n",
    )
    .unwrap();
    let (cd, out) = run(&dir, ["doctor", "--json"]);
    // Broken refs are a warning, not a failure.
    assert_eq!(cd, 0, "broken refs warn only: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let refs = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "patterns refs")
        .expect("doctor must include patterns refs");
    assert_eq!(refs["status"], "warn");
    assert!(refs["detail"].as_str().unwrap().contains("does-not-exist"));
}
