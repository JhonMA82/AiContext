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
        "aicontext-tools-{name}-{}-{n}-{:?}",
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
fn plan_json_contract_and_sections() {
    let dir = fixture_repo("node-single");
    let (c, out) = run(&dir, ["tools", "plan", "--json"]);
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).expect("plan --json must be JSON");
    assert_eq!(v["schema"], "aicontext/tools-plan/v1");
    let titles: Vec<&str> = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["title"].as_str())
        .collect();
    assert!(titles.contains(&"Required"));
    assert!(titles.contains(&"Core"));
    assert!(titles.contains(&"Recommended"));
    // git is required and present in every test env.
    let git_entry = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["entries"].as_array().unwrap().clone())
        .find(|e| e["tool"] == "git")
        .expect("plan must include git");
    assert_eq!(git_entry["state"], "available");
}

#[test]
fn plan_is_read_only() {
    let dir = fixture_repo("bun-monorepo");
    // No init: plan must work from scan alone and create nothing.
    let (c, _) = run(&dir, ["tools", "plan"]);
    assert_eq!(c, 0);
    assert!(
        !dir.join(".engineering").exists(),
        "tools plan must not create .engineering"
    );
    let (c2, _) = run(&dir, ["tools", "status"]);
    assert_eq!(c2, 0);
    assert!(
        !dir.join(".engineering").exists(),
        "tools status must not create .engineering"
    );
}

#[test]
fn status_json_contract() {
    let dir = fixture_repo("rust-single");
    let (c, out) = run(&dir, ["tools", "status", "--json"]);
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).expect("status --json must be JSON");
    assert_eq!(v["schema"], "aicontext/tools-status/v1");
    let names: Vec<&str> = v["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for expected in ["git", "ast-grep", "rg", "tgrep", "rtk", "codegraph"] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
}
