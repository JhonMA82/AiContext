//! AndMar Context consumer contract: one uniform flow over every repo kind.
//!
//! A single driver exercises only the consumer surface — `status`, `scan`,
//! `search`, `check` (all `--json`) plus reads of the `paths` files from
//! `status` — with NO branching on origin, topology, or recipe. The same
//! code must work on standalone monoliths/monorepos and on
//! Engineering-managed single/multi-surface projects (including unknown
//! recipes). Origin appears only in assertions about reported labels.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
}

fn run(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run aicontext");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(255), text)
}

fn run_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run aicontext");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let start = text.find('{').expect("json output");
    serde_json::from_str(&text[start..]).expect("parse json")
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

fn fresh_repo(name: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-ctx-{name}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    std::fs::create_dir_all(&base).expect("mkdir");
    base
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(&p, content).expect("write");
}

fn commit_all(dir: &Path, msg: &str) {
    git(dir, ["init", "-q"]);
    git(dir, ["add", "-A"]);
    git(dir, ["commit", "-qm", msg]);
}

fn write_engineering(dir: &Path, recipe: &str, surfaces: &[(&str, &str)]) {
    let mut components = String::new();
    let mut map_surfaces = String::new();
    for (i, (surface, dest)) in surfaces.iter().enumerate() {
        if i > 0 {
            components.push(',');
            map_surfaces.push(',');
        }
        components.push_str(&format!(
            r#"{{"surface":"{surface}","boilerplate":"mystery-bp","pin":"deadbeef","destination":"{dest}"}}"#
        ));
        map_surfaces.push_str(&format!(
            r#""{surface}":{{"path":"{dest}","provider":"mystery-bp","instructions":"{dest}/AGENTS.md"}}"#
        ));
    }
    write(
        dir,
        ".engineering/project.json",
        &format!(
            r#"{{"schema_version":2,"project":"demo","recipe":"{recipe}","catalog_version":"1.1.0","plan_fingerprint":"abc123","components":[{components}],"files":[]}}"#
        ),
    );
    write(
        dir,
        ".engineering/project-map.json",
        &format!(r#"{{"schema_version":1,"surfaces":{{{map_surfaces}}},"relationships":[]}}"#),
    );
    write(
        dir,
        ".engineering/provenance.json",
        r#"{"schema_version":"2","core_version":"9.9.9","catalog_version":"1.1.0","plan_fingerprint":"abc123","pins":{"mystery-bp":"deadbeef"},"materialized_at":"2026-01-01T00:00:00Z"}"#,
    );
    write(dir, "AGENTS.md", "# demo — agent router\n");
    for (_, dest) in surfaces {
        std::fs::create_dir_all(dir.join(dest)).expect("mkdir dest");
    }
}

/// The uniform consumer flow. No origin/topology/recipe branching: status →
/// minimum context (paths files) → targeted search (scoped when the scan
/// reports subprojects) → consistency check. Returns the origin label for
/// the caller to assert.
fn consumer_flow(dir: &Path, token: &str) -> String {
    let _ = run(dir, &["init", "--non-interactive"]);
    let (cs, sout) = run(dir, &["sync"]);
    assert_eq!(cs, 0, "sync must converge: {sout}");

    // 1. status: minimum repository context envelope.
    let status = run_json(dir, &["status", "--json"]);
    for key in ["schema", "repo", "state", "head", "complexity", "last_check", "subprojects", "origin"] {
        assert!(status.get(key).is_some(), "status lacks {key}: {status}");
    }
    // 2. Minimum context via the resolved paths (never hardcoded).
    let ps = status["paths"]["project_state"].as_str().expect("paths.project_state");
    let pat = status["paths"]["patterns"].as_str().expect("paths.patterns");
    let state_text = std::fs::read_to_string(dir.join(ps)).expect("read project_state");
    let patterns_text = std::fs::read_to_string(dir.join(pat)).expect("read patterns");
    assert!(!state_text.trim().is_empty(), "project state must be readable");
    assert!(!patterns_text.trim().is_empty(), "patterns must be readable");

    // 3. Targeted search, scoped through the scan-reported routing input.
    // The token is seeded into the scoped location so the assertion is
    // meaningful whichever subproject sorts first.
    let scan = run_json(dir, &["scan", "--json"]);
    let subprojects = scan["subprojects"].as_array().expect("subprojects array");
    let search = if let Some(first) = subprojects.first() {
        let path = first["path"].as_str().expect("subproject path");
        write(dir, &format!("{path}/note.txt"), &format!("{token}\n"));
        git(dir, ["add", "-A"]);
        run_json(dir, &["search", token, "--subproject", path, "--json"])
    } else {
        write(dir, "note.txt", &format!("{token}\n"));
        git(dir, ["add", "-A"]);
        run_json(dir, &["search", token, "--json"])
    };
    for key in ["schema", "query", "mode", "backend", "knowledge", "hits"] {
        assert!(search.get(key).is_some(), "search lacks {key}: {search}");
    }
    assert_eq!(search["query"], token);
    let hit_text: String = search["hits"]
        .as_array()
        .map(|h| h.iter().filter_map(|x| x.get("text")).map(|t| t.to_string()).collect())
        .unwrap_or_default();
    assert!(hit_text.contains(token), "search must find the token: {search}");

    // 4. Consistency check envelope (observed, not necessarily green).
    let check = run_json(dir, &["check", "--json"]);
    assert_eq!(check["schema"], "aicontext/check/v1");
    assert!(check.get("passed").and_then(|p| p.as_bool()).is_some());
    for f in check["findings"].as_array().expect("findings array") {
        assert!(f.get("name").and_then(|n| n.as_str()).is_some());
        assert!(f.get("passed").and_then(|p| p.as_bool()).is_some());
        assert!(f.get("detail").and_then(|d| d.as_str()).is_some());
    }

    status["origin"].as_str().unwrap_or("?").to_string()
}

#[test]
fn uniform_flow_standalone_monolith() {
    let dir = fresh_repo("mono");
    write(&dir, "package.json", r#"{"name":"demo","version":"0.1.0"}"#);
    commit_all(&dir, "fixture");
    assert_eq!(consumer_flow(&dir, "ctxtok-mono"), "standalone");
    let status = run_json(&dir, &["status", "--json"]);
    assert!(status.get("engineering").is_none());
}

#[test]
fn uniform_flow_standalone_monorepo() {
    let dir = fresh_repo("monorepo");
    write(&dir, "package.json", r#"{"name":"root","version":"0.1.0","workspaces":["apps/*"]}"#);
    write(&dir, "apps/web/package.json", r#"{"name":"web","version":"0.1.0"}"#);
    write(&dir, "apps/api/package.json", r#"{"name":"api","version":"0.1.0"}"#);
    commit_all(&dir, "fixture");
    assert_eq!(consumer_flow(&dir, "ctxtok-mr"), "standalone");
}

#[test]
fn uniform_flow_engineering_single_surface() {
    let dir = fresh_repo("eng1");
    write_engineering(&dir, "GP-02", &[("web-admin", "apps/admin")]);
    write(&dir, "package.json", r#"{"name":"demo","version":"0.1.0"}"#);
    commit_all(&dir, "fixture");
    assert_eq!(consumer_flow(&dir, "ctxtok-e1"), "engineering-platform");
    let status = run_json(&dir, &["status", "--json"]);
    assert_eq!(status["engineering"]["recipe"], "GP-02");
}

#[test]
fn uniform_flow_engineering_unknown_recipe() {
    let dir = fresh_repo("engx");
    write_engineering(&dir, "FUTURE-42", &[("quantum", "q-front"), ("neural", "q-back")]);
    write(&dir, "package.json", r#"{"name":"demo","version":"0.1.0"}"#);
    commit_all(&dir, "fixture");
    assert_eq!(consumer_flow(&dir, "ctxtok-ex"), "engineering-platform");
    let status = run_json(&dir, &["status", "--json"]);
    assert_eq!(status["engineering"]["recipe"], "FUTURE-42");
}
