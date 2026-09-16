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

fn run_with_path<const N: usize>(dir: &Path, args: [&str; N], path: String) -> (i32, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .env("PATH", path)
        .output()
        .expect("run aicontext");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(255), text)
}

#[test]
fn doctor_warns_on_unindexed_tgrep_dir() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    // A .tgrep/ dir whose `status` prints the exit-0 "No index found" message
    // (real tgrep 1.x behavior) must warn, never report a healthy Ok.
    std::fs::create_dir_all(dir.join(".tgrep")).unwrap();
    let bindir = dir.join("shim-bin");
    std::fs::create_dir_all(&bindir).unwrap();
    let script = r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "tgrep test-0.0.0"; exit 0; fi
echo "No index found at ./.tgrep"
echo "Run `tgrep index .` to build one."
exit 0
"#;
    std::fs::write(bindir.join("tgrep"), script).expect("shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = bindir.join("tgrep");
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
    let path = format!(
        "{}:{}",
        bindir.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    );
    let (cd, out) = run_with_path(&dir, ["doctor", "--json"], path);
    assert_eq!(cd, 0, "a warning must not fail doctor: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let diag = v
        .get("diagnostics")
        .and_then(|d| d.as_array())
        .and_then(|ds| {
            ds.iter()
                .find(|d| d.get("name").and_then(|n| n.as_str()) == Some("tgrep index"))
        })
        .expect("doctor must include tgrep index");
    assert_eq!(diag.get("status").and_then(|s| s.as_str()), Some("warn"));
    assert!(
        diag.get("detail")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .contains("corrupt"),
        "detail must admit a corrupt index: {}",
        diag.get("detail")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    );
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
    let mut names: Vec<&str> = Vec::new();
    if let Some(ds) = v.get("diagnostics").and_then(|d| d.as_array()) {
        for d in ds {
            if let Some(n) = d.get("name").and_then(|n| n.as_str()) {
                names.push(n);
            }
        }
    }
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
    let config = v
        .get("diagnostics")
        .and_then(|d| d.as_array())
        .and_then(|ds| {
            ds.iter()
                .find(|d| d.get("name").and_then(|n| n.as_str()) == Some("config"))
        })
        .expect("doctor must include a config diagnostic");
    assert_eq!(config.get("status").and_then(|s| s.as_str()), Some("fail"));
    assert!(
        config
            .get("remediation")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .contains("init"),
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
    let refs = v
        .get("diagnostics")
        .and_then(|d| d.as_array())
        .and_then(|ds| {
            ds.iter()
                .find(|d| d.get("name").and_then(|n| n.as_str()) == Some("patterns refs"))
        })
        .expect("doctor must include patterns refs");
    assert_eq!(refs.get("status").and_then(|s| s.as_str()), Some("warn"));
    assert!(refs
        .get("detail")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .contains("does-not-exist"));
}
