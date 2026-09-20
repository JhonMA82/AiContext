//! Subproject detection contract (`aicontext/scan/v2`).
//!
//! These tests lock the deterministic behavior of `detect_subprojects`:
//! declared workspaces win over container scans, cargo `members` are parsed
//! as real paths, wildcard forms the detector cannot resolve are reported as
//! unresolved instead of guessed, and child scripts never leak into the root
//! command list. All fixtures are committed content; nothing here touches the
//! network.

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

/// Unique temp dir per call, mirroring `tests/p0.rs`. Parallel tests may use
/// the same fixture name, so the process id, thread id and a sequence number
/// are all folded into the path.
fn fixture_repo(name: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-subprojects-{name}-{}-{n}-{:?}",
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

/// Builds a synthetic temp repo from `(relative path, contents)` pairs. Used
/// for layouts too small to deserve a committed fixture.
fn repo_with(files: &[(&str, &str)]) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-subprojects-synth-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    for (rel, content) in files {
        let path = base.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, content).expect("write");
    }
    git(&base, ["init", "-q"]);
    git(&base, ["add", "-A"]);
    git(&base, ["commit", "-qm", "fixture"]);
    base
}

fn scan(dir: &Path) -> serde_json::Value {
    let (code, out) = run(dir, ["scan", "--json"]);
    assert_eq!(code, 0, "scan --json must succeed: {out}");
    serde_json::from_str(&out).expect("scan --json must be JSON")
}

fn sub_paths(v: &serde_json::Value) -> Vec<&str> {
    v["subprojects"]
        .as_array()
        .expect("subprojects must be an array")
        .iter()
        .map(|s| s["path"].as_str().expect("path must be a string"))
        .collect()
}

fn find<'a>(v: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    v["subprojects"]
        .as_array()
        .expect("subprojects")
        .iter()
        .find(|s| s["path"].as_str() == Some(path))
        .unwrap_or_else(|| panic!("missing subproject {path}"))
}

#[test]
fn monorepo_full_detects_four_sorted_subprojects() {
    let dir = fixture_repo("monorepo-full");
    let v = scan(&dir);
    assert_eq!(
        sub_paths(&v),
        ["apps/desktop", "apps/mobile", "apps/web", "services/api"]
    );
    assert_eq!(v["subprojects_source"]["mode"], "declared");

    for path in ["apps/desktop", "apps/mobile", "apps/web", "services/api"] {
        let s = find(&v, path);
        assert_eq!(s["kind"], "manifest", "{path} is manifest-backed");
        assert_eq!(s["agents_md"], true, "{path} has an AGENTS.md");
        assert_eq!(s["initialized"], false, "{path} has no .engineering yet");
    }

    let desktop = find(&v, "apps/desktop");
    assert_eq!(desktop["manifest"], "package.json");
    assert_eq!(desktop["manager"], "npm");
    assert_eq!(desktop["name"], "@fixture/desktop");

    let api = find(&v, "services/api");
    assert_eq!(api["manifest"], "Cargo.toml");
    assert_eq!(api["manager"], "cargo");
    assert_eq!(api["name"], "api-service");
}

#[test]
fn cargo_workspace_members_are_parsed() {
    let dir = fixture_repo("polyglot-monorepo");
    let v = scan(&dir);
    assert_eq!(sub_paths(&v), ["services/api", "services/worker"]);
    assert_eq!(v["subprojects_source"]["mode"], "declared");
    // apps/web has a package.json but is not a declared member: declared wins
    // and the container scan never runs.
    assert!(
        !sub_paths(&v).contains(&"apps/web"),
        "undeclared apps/web must not be detected"
    );
    assert_eq!(find(&v, "services/api")["name"], "api");
    assert_eq!(find(&v, "services/worker")["name"], "worker");
}

#[test]
fn containers_fallback_when_nothing_is_declared() {
    let dir = repo_with(&[("apps/web/package.json", "{\"name\":\"@t/web\"}")]);
    let v = scan(&dir);
    assert_eq!(v["subprojects_source"]["mode"], "containers");
    assert_eq!(
        v["subprojects_source"]["containers"],
        serde_json::json!(["apps"])
    );
    assert_eq!(sub_paths(&v), ["apps/web"]);
    assert_eq!(find(&v, "apps/web")["manager"], "npm");
}

#[test]
fn agents_marker_without_manifest_is_detected() {
    let dir = repo_with(&[("apps/web/AGENTS.md", "# Web\n")]);
    let v = scan(&dir);
    let s = find(&v, "apps/web");
    assert_eq!(s["kind"], "agents-marker");
    assert_eq!(s["manifest"], serde_json::Value::Null);
    assert_eq!(s["manager"], serde_json::Value::Null);
    assert_eq!(s["name"], "web");
    assert_eq!(s["agents_md"], true);
}

#[test]
fn excluded_dirs_are_never_subprojects() {
    let dir = fixture_repo("monorepo-full");
    let nested = dir.join("apps/web/node_modules/dep");
    std::fs::create_dir_all(&nested).expect("mkdir node_modules");
    std::fs::write(nested.join("package.json"), "{\"name\":\"dep\"}").expect("write");
    let v = scan(&dir);
    assert_eq!(
        sub_paths(&v),
        ["apps/desktop", "apps/mobile", "apps/web", "services/api"]
    );
    assert!(
        !sub_paths(&v).iter().any(|p| p.contains("node_modules")),
        "node_modules must never be a subproject"
    );
}

#[test]
fn single_project_repos_detect_nothing() {
    for fixture in ["node-single", "rust-single"] {
        let dir = fixture_repo(fixture);
        let v = scan(&dir);
        assert!(
            sub_paths(&v).is_empty(),
            "{fixture} is a single project, got {:?}",
            sub_paths(&v)
        );
    }
}

#[test]
fn apps_dir_without_manifests_yields_nothing() {
    let dir = fixture_repo("apps-no-projects");
    let v = scan(&dir);
    assert!(sub_paths(&v).is_empty());
    assert_eq!(v["subprojects_source"]["mode"], "none");
}

#[test]
fn child_commands_do_not_leak_into_root_commands() {
    let dir = fixture_repo("monorepo-full");
    let v = scan(&dir);
    let root: Vec<&str> = v["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .filter_map(|c| c.as_str())
        .collect();
    assert!(
        root.contains(&"root:check"),
        "root own script stays: {root:?}"
    );
    assert!(
        !root.contains(&"build") && !root.contains(&"dev"),
        "child-only scripts must not enter root commands: {root:?}"
    );
    let web: Vec<&str> = find(&v, "apps/web")["commands"]
        .as_array()
        .expect("child commands")
        .iter()
        .filter_map(|c| c.as_str())
        .collect();
    assert_eq!(web, ["build", "dev"], "child commands live on the child");
}

#[test]
fn unresolved_globs_are_reported_not_guessed() {
    let dir = repo_with(&[
        (
            "package.json",
            "{\"name\":\"guesser\",\"workspaces\":[\"apps/**\"]}",
        ),
        ("apps/web/package.json", "{\"name\":\"@t/web\"}"),
    ]);
    let v = scan(&dir);
    assert_eq!(
        v["subprojects_source"]["unresolved"],
        serde_json::json!(["apps/**"])
    );
    assert_eq!(
        v["subprojects_source"]["declared"],
        serde_json::json!(["apps/**"])
    );
}

#[test]
fn scan_v2_contract() {
    let dir = fixture_repo("node-single");
    let v = scan(&dir);
    assert_eq!(v["schema"], "aicontext/scan/v2");
    for field in [
        "schema",
        "git",
        "languages",
        "packages",
        "commands",
        "tools",
        "complexity",
        "version_candidates",
        "docs",
        "ci",
        "subprojects",
        "subprojects_source",
    ] {
        assert!(
            v.get(field).is_some(),
            "scan/v2 must carry required field {field}"
        );
    }
}
