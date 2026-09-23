//! Cross-compatibility with Engineering Platform (self-contained fixtures).
//!
//! Covers the 13 cross-repo scenarios without depending on any absolute
//! path: every fixture is built in a unique temp dir with a git repo.
//! Scenario 6 (Engineering operations preserve AiContext state) is covered
//! on the Go side by `internal/project/ownership_test.go`; here the mirror
//! direction is verified (AiContext never touches Engineering artifacts).

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
    assert!(
        out.status.success() || out.status.code() == Some(1),
        "command {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    // check/status print pretty JSON to stdout; find the JSON object.
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
        "aicontext-eng-{name}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    std::fs::create_dir_all(&base).expect("mkdir");
    base
}

fn commit_all(dir: &Path, msg: &str) {
    git(dir, ["init", "-q"]);
    git(dir, ["add", "-A"]);
    git(dir, ["commit", "-qm", msg]);
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(&p, content).expect("write");
}

fn root_package_json() -> &'static str {
    r#"{"name":"demo","version":"0.1.0"}"#
}

fn sub_package_json(name: &str) -> String {
    format!(r#"{{"name":"{name}","version":"0.1.0","scripts":{{"build":"echo"}}}}"#)
}

/// Minimal but well-formed Engineering contracts with an UNKNOWN recipe and
/// UNKNOWN surfaces (no recipe hardcoding allowed to matter).
fn write_engineering(dir: &Path, recipe: &str, surfaces: &[(&str, &str)]) {
    // surfaces: (logical surface, destination path)
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
    write(
        dir,
        ".engineering/implementation-brief.md",
        "# Brief (Engineering-owned)\n",
    );
    write(
        dir,
        ".engineering/handoff.json",
        r#"{"schema_version":1,"status":"ready_for_implementation","next_owner":"gentle-ai","locked":["architecture"],"requirements":[],"open_questions":[]}"#,
    );
    write(
        dir,
        "ARCHITECTURE.md",
        "# Architecture (Engineering-owned)\n",
    );
    write(dir, "GENTLE.md", "# Gentle (Engineering-owned)\n");
    write(
        dir,
        "AGENTS.md",
        "# demo — agent router\n\nEngineering-owned router content.\n",
    );
    for (_, dest) in surfaces {
        write(
            dir,
            &format!("{dest}/package.json"),
            &sub_package_json(dest),
        );
        std::fs::create_dir_all(dir.join(dest)).expect("mkdir dest");
    }
    write(dir, "package.json", root_package_json());
}

// First init on a repo where routing is created reports stale (the new
// root AGENTS.md joins the generated `Important paths` list); `sync`
// converges. This is the established init → sync → green convention.
fn init_then_sync(dir: &Path) {
    let (_, out) = run(dir, &["init", "--non-interactive"]);
    let (cs, sout) = run(dir, &["sync"]);
    assert_eq!(cs, 0, "sync must converge after init: {out} / {sout}");
}

fn check_green(dir: &Path) -> serde_json::Value {
    let check = run_json(dir, &["check", "--json"]);
    assert_eq!(check["passed"], true, "check must pass: {check}");
    check
}

fn finding<'a>(check: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    check["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("missing finding {name}: {}", check))
}

// 1. Standalone monolith adoption keeps working.
#[test]
fn standalone_monolith_adoption() {
    let dir = fresh_repo("mono1");
    write(&dir, "package.json", root_package_json());
    write(&dir, "README.md", "# demo\n");
    commit_all(&dir, "fixture");
    let (c, _) = run(&dir, &["init", "--non-interactive"]);
    assert_eq!(c, 0, "init should pass on monolith");
    let scan = run_json(&dir, &["scan", "--json"]);
    assert!(scan.get("engineering").is_none(), "no engineering object");
    assert_ne!(
        scan["subprojects_source"]["mode"], "engineering",
        "standalone must not use engineering mode"
    );
    let check = run_json(&dir, &["check", "--json"]);
    assert_eq!(check["passed"], true);
    assert_eq!(finding(&check, "engineering")["passed"], true);
    assert!(finding(&check, "engineering")["detail"]
        .as_str()
        .unwrap()
        .contains("standalone"));
    let state = std::fs::read_to_string(dir.join(".engineering/PROJECT_STATE.md")).unwrap();
    assert!(
        !state.contains("Origin:"),
        "standalone state has no Origin lines"
    );
}

// 2. Standalone monorepo adoption keeps working.
#[test]
fn standalone_monorepo_adoption() {
    let dir = fresh_repo("mono2");
    write(
        &dir,
        "package.json",
        r#"{"name":"root","version":"0.1.0","workspaces":["apps/*"]}"#,
    );
    write(&dir, "apps/web/package.json", &sub_package_json("web"));
    write(&dir, "apps/api/package.json", &sub_package_json("api"));
    commit_all(&dir, "fixture");
    init_then_sync(&dir);
    let scan = run_json(&dir, &["scan", "--json"]);
    assert_eq!(scan["subprojects_source"]["mode"], "declared");
    assert_eq!(scan["subprojects"].as_array().unwrap().len(), 2);
    check_green(&dir);
}

// 3. Engineering-managed repositories are detected.
#[test]
fn detects_engineering_managed() {
    let dir = fresh_repo("eng3");
    write_engineering(&dir, "GP-99", &[("alpha", "frontend"), ("beta", "backend")]);
    commit_all(&dir, "fixture");
    // Detection works before init (scan is read-only).
    let scan = run_json(&dir, &["scan", "--json"]);
    let eng = &scan["engineering"];
    assert_eq!(eng["origin"], "engineering-platform");
    assert_eq!(eng["supported"], true);
    assert_eq!(eng["recipe"], "GP-99");
    assert_eq!(scan["subprojects_source"]["mode"], "engineering");
    init_then_sync(&dir);
    let status = run_json(&dir, &["status", "--json"]);
    assert_eq!(status["origin"], "engineering-platform");
    assert_eq!(status["engineering"]["recipe"], "GP-99");
}

// 4. Project-map has precedence over equivalent rediscovery.
#[test]
fn project_map_has_precedence() {
    let dir = fresh_repo("eng4");
    // Destinations outside every default container and with no workspace
    // declarations: standalone detection finds nothing here.
    write_engineering(&dir, "GP-99", &[("alpha", "frontend"), ("beta", "backend")]);
    commit_all(&dir, "fixture");
    let scan = run_json(&dir, &["scan", "--json"]);
    assert_eq!(scan["subprojects_source"]["mode"], "engineering");
    let paths: Vec<String> = scan["subprojects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(paths, vec!["backend".to_string(), "frontend".to_string()]);
    for s in scan["subprojects"].as_array().unwrap() {
        assert_eq!(s["kind"], "engineering-surface");
    }
}

// 5. AiContext never overwrites Engineering-owned artifacts.
#[test]
fn never_overwrites_engineering_artifacts() {
    let dir = fresh_repo("eng5");
    write_engineering(&dir, "GP-99", &[("alpha", "frontend"), ("beta", "backend")]);
    commit_all(&dir, "fixture");
    let before: Vec<(String, String)> = [
        ".engineering/project.json",
        ".engineering/project-map.json",
        ".engineering/provenance.json",
        ".engineering/implementation-brief.md",
        ".engineering/handoff.json",
        "ARCHITECTURE.md",
        "GENTLE.md",
    ]
    .iter()
    .map(|f| (f.to_string(), std::fs::read_to_string(dir.join(f)).unwrap()))
    .collect();
    for cmd in [
        ["init", "--non-interactive"],
        ["sync", ""],
        ["check", ""],
        ["doctor", ""],
    ] {
        let args: Vec<&str> = cmd.iter().filter(|s| !s.is_empty()).copied().collect();
        let _ = run(&dir, args.as_slice());
    }
    for (f, content) in &before {
        assert_eq!(
            &std::fs::read_to_string(dir.join(f)).unwrap(),
            content,
            "{f} must be byte-identical"
        );
    }
}

// 7. AGENTS.md keeps both systems' content (8. drift variant below reuses it).
#[test]
fn agents_md_keeps_both_systems() {
    let dir = fresh_repo("eng7");
    write_engineering(&dir, "GP-99", &[("alpha", "frontend"), ("beta", "backend")]);
    commit_all(&dir, "fixture");
    init_then_sync(&dir);
    let agents = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("Engineering-owned router content."),
        "engineering bytes preserved"
    );
    assert!(
        agents.contains("<!-- aicontext:routing:start -->"),
        "routing block added"
    );
    assert!(
        agents.contains("<!-- aicontext:context:start -->"),
        "context block added"
    );
    // Sync only refreshes inside the markers.
    let (c2, _) = run(&dir, &["sync"]);
    assert_eq!(c2, 0);
    let agents2 = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(agents2.contains("Engineering-owned router content."));
    // Surface AGENTS.md is never created by AiContext.
    assert!(!dir.join("frontend/AGENTS.md").exists());
}

// 8. Filesystem vs Engineering declaration drift is reported.
#[test]
fn filesystem_drift_is_reported() {
    let dir = fresh_repo("eng8");
    write_engineering(&dir, "GP-99", &[("alpha", "frontend"), ("beta", "backend")]);
    commit_all(&dir, "fixture");
    init_then_sync(&dir);
    std::fs::remove_dir_all(dir.join("backend")).expect("remove declared dir");
    let check = run_json(&dir, &["check", "--json"]);
    assert_eq!(check["passed"], false, "drift must fail check: {check}");
    let eng = finding(&check, "engineering");
    assert_eq!(eng["passed"], false);
    assert!(eng["detail"].as_str().unwrap().contains("backend"));
}

// 9 + 10. Engineering evolution causes drift; sync converges on
// deterministic-only changes.
#[test]
fn evolution_drift_and_sync_convergence() {
    let dir = fresh_repo("eng910");
    write_engineering(&dir, "GP-99", &[("alpha", "frontend"), ("beta", "backend")]);
    commit_all(&dir, "fixture");
    init_then_sync(&dir);
    // Deterministic-only evolution: rotate the plan fingerprint in BOTH
    // manifest and provenance (topology unchanged).
    for f in [".engineering/project.json", ".engineering/provenance.json"] {
        let p = dir.join(f);
        let text = std::fs::read_to_string(&p)
            .unwrap()
            .replace("abc123", "def456");
        std::fs::write(&p, text).unwrap();
    }
    // Drift is visible before sync.
    let (sync_check_code, _) = run(&dir, &["sync", "--check"]);
    assert_eq!(sync_check_code, 1, "generated block must be stale");
    let check = run_json(&dir, &["check", "--json"]);
    assert_eq!(
        check["passed"], false,
        "stale state must fail check: {check}"
    );
    // Sync updates only deterministic facts and converges.
    let (sync_code, sout) = run(&dir, &["sync"]);
    assert_eq!(sync_code, 0, "sync must converge: {sout}");
    let check2 = run_json(&dir, &["check", "--json"]);
    assert_eq!(
        check2["passed"], true,
        "deterministic change converges: {check2}"
    );
    let status = run_json(&dir, &["status", "--json"]);
    assert_eq!(status["state"], "synchronized");
}

// 11. Semantic knowledge is not regenerated unnecessarily.
#[test]
fn semantic_knowledge_survives_sync() {
    let dir = fresh_repo("eng11");
    write_engineering(&dir, "GP-99", &[("alpha", "frontend"), ("beta", "backend")]);
    commit_all(&dir, "fixture");
    init_then_sync(&dir);
    // Simulate skill output: curated state, patterns, subproject purposes.
    let state_path = dir.join(".engineering/PROJECT_STATE.md");
    let state = std::fs::read_to_string(&state_path).unwrap();
    let with_curated = state.replace("TBD", "Curated semantic knowledge.");
    std::fs::write(&state_path, with_curated).unwrap();
    write(
        &dir,
        ".engineering/PATTERNS.md",
        "# Patterns\n\n## Real pattern\n\nStatus: observed\n",
    );
    let manifest = std::fs::read_to_string(dir.join(".engineering/subprojects.yml")).unwrap();
    let with_purpose = manifest.replace("purpose: \"\"", "purpose: \"does things\"");
    std::fs::write(dir.join(".engineering/subprojects.yml"), with_purpose).unwrap();
    let (c2, _) = run(&dir, &["sync"]);
    assert_eq!(c2, 0);
    let state2 = std::fs::read_to_string(&state_path).unwrap();
    assert!(state2.contains("Curated semantic knowledge."));
    assert!(state2.contains("Origin: engineering-platform"));
    let patterns = std::fs::read_to_string(dir.join(".engineering/PATTERNS.md")).unwrap();
    assert!(patterns.contains("Real pattern"));
    let manifest2 = std::fs::read_to_string(dir.join(".engineering/subprojects.yml")).unwrap();
    assert!(manifest2.contains("does things"));
    check_green(&dir);
}

// 12. Unknown recipes/surfaces work while respecting the contract.
#[test]
fn unknown_recipe_and_surfaces_work() {
    let dir = fresh_repo("eng12");
    write_engineering(
        &dir,
        "FUTURE-42",
        &[("quantum", "q-front"), ("neural", "q-back")],
    );
    commit_all(&dir, "fixture");
    let scan = run_json(&dir, &["scan", "--json"]);
    assert_eq!(scan["engineering"]["recipe"], "FUTURE-42");
    assert_eq!(scan["subprojects_source"]["mode"], "engineering");
    init_then_sync(&dir);
    check_green(&dir);
    // Scoped search routes through the declared path.
    write(&dir, "q-front/note.txt", "needle-here\n");
    git(&dir, ["add", "-A"]);
    let (sc, out) = run(&dir, &["search", "needle-here", "--subproject", "q-front"]);
    assert_eq!(sc, 0);
    assert!(out.contains("needle-here"));
}

// 13. Absence of Engineering metadata keeps standalone behavior.
#[test]
fn absence_keeps_standalone_behavior() {
    let dir = fresh_repo("eng13");
    write(&dir, "package.json", root_package_json());
    commit_all(&dir, "fixture");
    let scan = run_json(&dir, &["scan", "--json"]);
    assert!(scan.get("engineering").is_none());
    let status = run_json(&dir, &["status", "--json"]);
    assert!(status.get("origin").is_none() || status["origin"] == "standalone");
    let state =
        std::fs::read_to_string(dir.join(".engineering/PROJECT_STATE.md")).unwrap_or_default();
    let _ = state;
    let (c, _) = run(&dir, &["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let state2 = std::fs::read_to_string(dir.join(".engineering/PROJECT_STATE.md")).unwrap();
    assert!(!state2.contains("Origin:"));
}

// Unsupported contracts fail closed instead of silently passing as standalone.
#[test]
fn unsupported_contracts_fail_closed() {
    let dir = fresh_repo("engU");
    write(&dir, "package.json", root_package_json());
    write(
        &dir,
        ".engineering/project.json",
        r#"{"schema_version":99,"project":"demo","recipe":"GP-99","catalog_version":"1.1.0","plan_fingerprint":"x","components":[{"surface":"a","boilerplate":"b","pin":"c","destination":"frontend"}],"files":[]}"#,
    );
    write(
        &dir,
        ".engineering/project-map.json",
        r#"{"schema_version":1,"surfaces":{"a":{"path":"frontend","provider":"b","instructions":"frontend/AGENTS.md"}},"relationships":[]}"#,
    );
    commit_all(&dir, "fixture");
    let scan = run_json(&dir, &["scan", "--json"]);
    assert_eq!(scan["engineering"]["supported"], false);
    let (c, _) = run(&dir, &["init", "--non-interactive"]);
    let _ = c;
    let check = run_json(&dir, &["check", "--json"]);
    assert_eq!(finding(&check, "engineering")["passed"], false);
}
