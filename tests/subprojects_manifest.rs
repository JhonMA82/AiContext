//! Seed-once `subprojects.yml` adoption tracker (`aicontext/subprojects/v1`)
//! and its `check` shape gate: `init` seeds every detected path as
//! `status: pending`, neither `init` nor `sync` ever rewrites an existing
//! file, and `check` fails closed on unknown keys and on status values
//! outside the closed `pending`/`adopted` enum. Detection reconciliation
//! (missing/stale entries, the adopted lifecycle) belongs to WU4, not to
//! this shape gate. All fixtures are copied into temp repositories (the
//! committed fixtures are read-only); nothing here touches the network.

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

fn temp_base(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-manifest-{tag}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    std::fs::create_dir_all(&base).expect("mkdir");
    base
}

fn fixture_repo(name: &str) -> PathBuf {
    let base = temp_base(name);
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    copy_dir(&manifest.join("fixtures").join(name), &base);
    git(&base, ["init", "-q"]);
    git(&base, ["add", "-A"]);
    git(&base, ["commit", "-qm", "fixture"]);
    base
}

fn init(dir: &Path) -> (i32, String) {
    run(dir, ["init", "--non-interactive"])
}

/// First `init` on a monorepo reports stale (creating `AGENTS.md` adds a
/// scanned doc, a pre-WU3 invariant locked by `tests/routing.rs`); `sync`
/// then converges. Tests that need a green tree use this helper.
fn init_converged(dir: &Path) -> String {
    let (_, out) = init(dir);
    let (cs, sout) = run(dir, ["sync"]);
    assert_eq!(cs, 0, "sync must converge after init: {sout}");
    out
}

fn manifest_text(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(".engineering/subprojects.yml")).expect("seeded manifest")
}

/// The `subprojects` finding of `check --json`, looked up by name.
fn subprojects_finding(dir: &Path) -> (i32, serde_json::Value) {
    let (code, out) = run(dir, ["check", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out)
        .unwrap_or_else(|e| panic!("check --json must be JSON ({e}): {out}"));
    let found = v["findings"]
        .as_array()
        .expect("findings must be an array")
        .iter()
        .find(|f| f["name"] == "subprojects")
        .unwrap_or_else(|| panic!("subprojects finding must exist: {v:?}"))
        .clone();
    (code, found)
}

/// Minimal contract check for `schemas/subprojects-v1.json`: `required`,
/// `const`, `enum`, `type` and `additionalProperties: false` on objects.
/// Deliberately not a general JSON-schema implementation; it locks the
/// fields this work emits.
fn assert_matches_schema(v: &serde_json::Value, file: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("schemas")
        .join(file);
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}")),
    )
    .expect("schema must be JSON");
    assert_prop(v, &schema, "$");
}

fn assert_prop(value: &serde_json::Value, schema: &serde_json::Value, prefix: &str) {
    if let Some(reqs) = schema["required"].as_array() {
        for req in reqs {
            let key = req.as_str().expect("required key");
            assert!(
                value.get(key).is_some(),
                "{prefix}: missing required field {key}"
            );
        }
    }
    if let Some(c) = schema.get("const") {
        assert_eq!(value, c, "{prefix}: const mismatch");
    }
    if let Some(serde_json::Value::Array(allowed)) = schema.get("enum") {
        assert!(
            allowed.contains(value),
            "{prefix}: {value} is not in the enum"
        );
    }
    if let Some(t) = schema.get("type").and_then(|t| t.as_str()) {
        let ok = match t {
            "object" => value.is_object(),
            "string" => value.is_string(),
            _ => true,
        };
        assert!(ok, "{prefix}: {value} does not match type {t}");
    }
    if schema.get("additionalProperties").and_then(|a| a.as_bool()) == Some(false) {
        if let Some(obj) = value.as_object() {
            let props = schema["properties"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            for key in obj.keys() {
                assert!(props.contains_key(key), "{prefix}: unknown key {key}");
            }
        }
    }
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        if let Some(obj) = value.as_object() {
            for (key, prop) in props {
                if key == "subprojects" {
                    // Map values share one schema: check each entry.
                    if let Some(entries) = obj.get(key).and_then(|v| v.as_object()) {
                        let entry_schema = &prop["additionalProperties"];
                        for (path, entry) in entries {
                            assert_prop(
                                entry,
                                entry_schema,
                                &format!("{prefix}.subprojects.{path}"),
                            );
                        }
                    }
                } else if let Some(child) = obj.get(key) {
                    assert_prop(child, prop, &format!("{prefix}.{key}"));
                }
            }
        }
    }
}

fn manifest_json(dir: &Path) -> serde_json::Value {
    let text = manifest_text(dir);
    let yml: serde_yaml::Value = serde_yaml::from_str(&text).expect("manifest must be YAML");
    serde_json::to_value(yml).expect("YAML must convert to JSON")
}

#[test]
fn init_seeds_detected_paths_as_pending() {
    let dir = fixture_repo("monorepo-full");
    let out = init_converged(&dir);
    assert!(out.contains("seeded with 4 entries"), "seed note: {out}");
    let text = manifest_text(&dir);
    let expected = "# Subproject adoption tracker (aicontext/subprojects/v1).\n# `init` seeds detected paths as `pending`; the adoption skill fills\n# `purpose` and moves entries to `adopted`. This file is never rewritten.\nschema: aicontext/subprojects/v1\nsubprojects:\n  apps/desktop:\n    purpose: \"\"\n    status: pending\n  apps/mobile:\n    purpose: \"\"\n    status: pending\n  apps/web:\n    purpose: \"\"\n    status: pending\n  services/api:\n    purpose: \"\"\n    status: pending\n";
    assert_eq!(text, expected, "seeded manifest must be byte-exact");
    assert_matches_schema(&manifest_json(&dir), "subprojects-v1.json");
}

#[test]
fn init_never_overwrites_existing_manifest() {
    let dir = fixture_repo("monorepo-full");
    std::fs::create_dir_all(dir.join(".engineering")).expect("mkdir");
    let custom = "schema: aicontext/subprojects/v1\nsubprojects:\n  apps/web:\n    purpose: Web app\n    status: adopted\n";
    std::fs::write(dir.join(".engineering/subprojects.yml"), custom).expect("write");
    let out = init_converged(&dir);
    assert!(out.contains("left intact"), "seed note: {out}");
    assert_eq!(
        manifest_text(&dir),
        custom,
        "existing manifest must survive init byte-identical"
    );
}

#[test]
fn sync_never_rewrites_manifest_but_projects_purpose() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    // The skill fills purpose/status; `sync` must project the purpose into
    // PROJECT_STATE without touching the manifest itself.
    let edited = manifest_text(&dir).replace(
        "services/api:\n    purpose: \"\"",
        "services/api:\n    purpose: API service",
    );
    let edited = edited.replace(
        "services/api:\n    purpose: API service\n    status: pending",
        "services/api:\n    purpose: API service\n    status: adopted",
    );
    std::fs::write(dir.join(".engineering/subprojects.yml"), &edited).expect("write");
    let (cs, _) = run(&dir, ["sync"]);
    assert_eq!(cs, 0, "sync must succeed");
    assert_eq!(
        manifest_text(&dir),
        edited,
        "sync must leave the manifest byte-identical"
    );
    let state = std::fs::read_to_string(dir.join(".engineering/PROJECT_STATE.md")).expect("state");
    assert!(
        state.contains("API service"),
        "purpose must be projected into PROJECT_STATE"
    );
    let (cs2, _) = run(&dir, ["sync"]);
    assert_eq!(cs2, 0, "second sync must converge");
}

#[test]
fn check_passes_seeded_manifest() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    let (code, found) = subprojects_finding(&dir);
    assert_eq!(code, 0, "check must pass: {found:?}");
    assert_eq!(found["passed"], true);
    assert!(
        found["detail"]
            .as_str()
            .unwrap_or("")
            .contains("4 subproject(s) validated"),
        "{found:?}"
    );
}

#[test]
fn check_rejects_unknown_status() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    let bad = manifest_text(&dir).replace("status: pending", "status: done");
    std::fs::write(dir.join(".engineering/subprojects.yml"), &bad).expect("write");
    let (code, found) = subprojects_finding(&dir);
    assert_eq!(code, 1, "unknown status must fail check");
    assert_eq!(found["passed"], false);
    let detail = found["detail"].as_str().unwrap_or("");
    assert!(
        detail.contains("apps/desktop"),
        "offender paths must be named: {detail}"
    );
    assert!(
        detail.contains("pending or adopted"),
        "closed enum must be named: {detail}"
    );
}

#[test]
fn check_rejects_unknown_keys() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    // Unknown field inside an entry.
    let bad = manifest_text(&dir).replace(
        "    status: pending",
        "    status: pending\n    owner: team-a",
    );
    std::fs::write(dir.join(".engineering/subprojects.yml"), &bad).expect("write");
    let (code, found) = subprojects_finding(&dir);
    assert_eq!(code, 1, "unknown entry field must fail check");
    assert_eq!(found["passed"], false);
    assert!(
        found["detail"].as_str().unwrap_or("").contains("unknown"),
        "{found:?}"
    );

    // Unknown top-level key.
    let bad_top = manifest_text(&dir).replace(
        "schema: aicontext/subprojects/v1",
        "schema: aicontext/subprojects/v1\nsubprojectz: {}",
    );
    std::fs::write(dir.join(".engineering/subprojects.yml"), &bad_top).expect("write");
    let (code, found) = subprojects_finding(&dir);
    assert_eq!(code, 1, "unknown top-level key must fail check");
    assert_eq!(found["passed"], false);
}

#[test]
fn check_rejects_unparseable_manifest() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    std::fs::write(
        dir.join(".engineering/subprojects.yml"),
        "schema: [unclosed",
    )
    .expect("write");
    let (code, found) = subprojects_finding(&dir);
    assert_eq!(code, 1, "unparseable manifest must fail closed");
    assert_eq!(found["passed"], false);
}

#[test]
fn no_manifest_without_detected_subprojects() {
    let dir = fixture_repo("node-single");
    let (c, out) = init(&dir);
    assert_eq!(c, 0, "init must pass: {out}");
    assert!(
        !dir.join(".engineering/subprojects.yml").exists(),
        "nothing detected: no manifest seeded"
    );
    let (code, found) = subprojects_finding(&dir);
    assert_eq!(code, 0, "absent manifest must pass check");
    assert_eq!(found["passed"], true);
    assert!(
        found["detail"]
            .as_str()
            .unwrap_or("")
            .contains("nothing to validate"),
        "{found:?}"
    );
}
