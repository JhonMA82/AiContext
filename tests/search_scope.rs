//! Scoped search contract (`aicontext/search/v1`) and `status` subproject
//! counts: `search --in <path>` restricts every backend to a repo-relative
//! path, `--subproject <path>` is `--in` after validating the path against
//! the detected subprojects, and `status` reports detected/adopted/pending.
//!
//! All fixtures are copied into temp repositories (the committed fixtures are
//! read-only); nothing here touches the network or the real repository.

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

fn run_with_env(dir: &Path, args: &[&str], path: &str, extra: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = Command::new(bin());
    cmd.args(args).current_dir(dir).env("PATH", path);
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run aicontext");
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

/// Unique temp dir per call, mirroring `tests/subprojects.rs` so parallel
/// tests never share a fixture copy.
fn unique_base(prefix: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-search-scope-{prefix}-{}-{n}-{:?}",
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
    let base = unique_base(name);
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    copy_dir(&manifest.join("fixtures").join(name), &base);
    git(&base, ["init", "-q"]);
    git(&base, ["add", "-A"]);
    git(&base, ["commit", "-qm", "fixture"]);
    base
}

/// Builds a synthetic temp repo from `(relative path, contents)` pairs.
fn repo_with(files: &[(&str, &str)]) -> PathBuf {
    let base = unique_base("synth");
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

fn search_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let (code, out) = run(dir, args);
    assert_eq!(code, 0, "search must succeed: {out}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("search --json must be JSON ({e}): {out}"))
}

fn hits_paths(v: &serde_json::Value) -> Vec<String> {
    v["hits"]
        .as_array()
        .expect("hits must be an array")
        .iter()
        .map(|h| h["path"].as_str().expect("hit path").to_string())
        .collect()
}

fn ast_grep_present() -> bool {
    Command::new("ast-grep")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// PATH containing only `git`, so tgrep/rg/ast-grep/codegraph probe as
/// missing and the text router must fall back to `git grep`.
fn git_only_path() -> PathBuf {
    let bindir = unique_base("git-only");
    let git_path = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("locate git");
    let git_bin = String::from_utf8_lossy(&git_path.stdout).trim().to_string();
    assert!(!git_bin.is_empty(), "git must exist for fixtures");
    std::os::unix::fs::symlink(&git_bin, bindir.join("git")).expect("link git");
    bindir
}

/// Minimal contract check for the schemas this file exercises: `required`
/// lists, `const`/`type`/`enum` on present properties and nested object
/// properties (the `subprojects` object of `status-v1`). Deliberately not a
/// general JSON-schema implementation; it locks the fields this work emits.
fn assert_matches_schema(v: &serde_json::Value, file: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("schemas")
        .join(file);
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}")),
    )
    .expect("schema must be JSON");
    assert_eq!(
        v["schema"].as_str(),
        schema["$id"].as_str(),
        "{file}: emitted schema id must match the contract"
    );
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
    let types: Vec<&str> = match schema.get("type") {
        Some(serde_json::Value::String(t)) => vec![t.as_str()],
        Some(serde_json::Value::Array(ts)) => ts.iter().filter_map(|t| t.as_str()).collect(),
        _ => Vec::new(),
    };
    if !types.is_empty() {
        assert!(
            types.iter().any(|t| type_matches(value, t)),
            "{prefix}: {value} does not match type {types:?}"
        );
    }
    if let Some(serde_json::Value::Object(props)) = schema.get("properties") {
        if let Some(obj) = value.as_object() {
            for (key, prop) in props {
                if let Some(child) = obj.get(key) {
                    assert_prop(child, prop, &format!("{prefix}.{key}"));
                }
            }
        }
    }
}

fn type_matches(value: &serde_json::Value, t: &str) -> bool {
    match t {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_u64().is_some(),
        "number" => value.is_number(),
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        _ => false,
    }
}

#[test]
fn in_scopes_text_hits_to_the_subproject() {
    let dir = fixture_repo("monorepo-full");
    // The literal lives in three subprojects: the unscoped run proves it.
    let all = search_json(&dir, &["search", "export const", "--json"]);
    let all_paths = hits_paths(&all);
    assert!(
        all_paths
            .iter()
            .any(|p| p.trim_start_matches("./").starts_with("apps/desktop/")),
        "unscoped hits must reach apps/desktop: {all_paths:?}"
    );
    assert!(
        all_paths
            .iter()
            .any(|p| p.trim_start_matches("./").starts_with("apps/web/")),
        "unscoped hits must reach apps/web: {all_paths:?}"
    );

    let scoped = search_json(
        &dir,
        &["search", "export const", "--in", "apps/web", "--json"],
    );
    assert_eq!(scoped["scope"], "apps/web");
    assert!(
        scoped.get("subproject").is_none(),
        "--in must not claim a subproject: {scoped:?}"
    );
    let paths = hits_paths(&scoped);
    assert!(
        !paths.is_empty(),
        "scoped search must still find the literal"
    );
    assert!(
        paths.iter().all(|p| p.starts_with("apps/web/")),
        "every hit must live under the scope: {paths:?}"
    );
}

#[test]
fn in_scopes_structural_hits() {
    let dir = fixture_repo("monorepo-full");
    if !ast_grep_present() {
        let (c, _) = run(
            &dir,
            &[
                "search",
                "export const $A = $B",
                "--structure",
                "--in",
                "apps/web",
            ],
        );
        assert_eq!(c, 3, "missing ast-grep must fail closed with exit 3");
        return;
    }
    let v = search_json(
        &dir,
        &[
            "search",
            "export const $A = $B",
            "--structure",
            "--in",
            "apps/web",
            "--json",
        ],
    );
    assert_eq!(v["mode"], "structure");
    assert_eq!(v["backend"], "ast-grep");
    assert_eq!(v["scope"], "apps/web");
    let paths = hits_paths(&v);
    assert!(
        !paths.is_empty(),
        "structural search must match the fixture"
    );
    assert!(
        paths.iter().all(|p| p.starts_with("apps/web/")),
        "ast-grep must stay inside the scope: {paths:?}"
    );
}

#[test]
fn subproject_flag_matches_in_scope() {
    let dir = fixture_repo("monorepo-full");
    let via_in = search_json(
        &dir,
        &["search", "export const", "--in", "apps/web", "--json"],
    );
    let mut via_sub = search_json(
        &dir,
        &[
            "search",
            "export const",
            "--subproject",
            "apps/web",
            "--json",
        ],
    );
    assert_eq!(via_sub["scope"], "apps/web");
    assert_eq!(via_sub["subproject"], "apps/web");
    via_sub
        .as_object_mut()
        .expect("JSON object")
        .remove("subproject");
    assert_eq!(
        via_in, via_sub,
        "--subproject X must equal --in X plus the subproject label"
    );

    // Human output carries no scope line, so it must be byte-identical.
    let (c1, human_in) = run(&dir, &["search", "export const", "--in", "apps/web"]);
    let (c2, human_sub) = run(
        &dir,
        &["search", "export const", "--subproject", "apps/web"],
    );
    assert_eq!(c1, 0, "{human_in}");
    assert_eq!(c2, 0, "{human_sub}");
    assert_eq!(human_in, human_sub);
}

#[test]
fn subproject_unknown_path_fails_with_candidates() {
    let dir = fixture_repo("monorepo-full");
    let (c, out) = run(
        &dir,
        &["search", "export const", "--subproject", "apps/ghost"],
    );
    assert_eq!(c, 5, "invalid scope uses the generic failure exit: {out}");
    assert!(out.contains("is not a detected subproject"), "{out}");
    for candidate in ["apps/desktop", "apps/mobile", "apps/web", "services/api"] {
        assert!(
            out.contains(candidate),
            "candidates must list {candidate}: {out}"
        );
    }
}

#[test]
fn in_nonexistent_path_fails() {
    let dir = fixture_repo("monorepo-full");
    let (c, out) = run(&dir, &["search", "export const", "--in", "apps/ghost"]);
    assert_eq!(c, 5, "{out}");
    assert!(out.contains("does not exist"), "{out}");

    // Absolute and escaping paths are rejected before touching the filesystem.
    let (c_abs, out_abs) = run(&dir, &["search", "export const", "--in", "/etc"]);
    assert_eq!(c_abs, 5, "{out_abs}");
    assert!(out_abs.contains("repo-relative"), "{out_abs}");
    let (c_up, out_up) = run(&dir, &["search", "export const", "--in", "../outside"]);
    assert_eq!(c_up, 5, "{out_up}");
    assert!(out_up.contains("repo-relative"), "{out_up}");
}

#[test]
fn scoped_search_is_deterministic() {
    let dir = fixture_repo("monorepo-full");
    let args = [
        "search",
        "export const web = \"web\"",
        "--in",
        "apps/web",
        "--json",
    ];
    let (c1, out1) = run(&dir, &args);
    let (c2, out2) = run(&dir, &args);
    assert_eq!(c1, 0, "{out1}");
    assert_eq!(c2, 0, "{out2}");
    assert_eq!(out1, out2, "two scoped runs must be byte-identical");
    let v: serde_json::Value = serde_json::from_str(&out1).expect("JSON");
    assert_eq!(
        hits_paths(&v),
        ["apps/web/src/index.ts"],
        "exactly the scoped hit"
    );
}

#[test]
fn status_json_counts_adopted_and_pending() {
    let dir = repo_with(&[
        ("apps/web/package.json", "{\"name\":\"@t/web\"}"),
        ("apps/api/package.json", "{\"name\":\"@t/api\"}"),
        (
            ".engineering/subprojects.yml",
            "schema: aicontext/subprojects/v1\n\
             subprojects:\n\
             \x20 apps/web:\n\
             \x20   purpose: Web app\n\
             \x20   status: adopted\n\
             \x20 apps/api:\n\
             \x20   purpose: API\n\
             \x20   status: pending\n\
             \x20 apps/ghost:\n\
             \x20   purpose: Ghost\n\
             \x20   status: pending\n",
        ),
    ]);
    let (c, out) = run(&dir, &["status", "--json"]);
    assert_eq!(c, 0, "status must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("status --json must be JSON");
    assert_eq!(v["schema"], "aicontext/status/v1");
    assert_eq!(
        v["subprojects"],
        serde_json::json!({"total": 2, "adopted": 1, "pending": 1}),
        "entries for non-detected paths are ignored"
    );
    assert_matches_schema(&v, "status-v1.json");

    let (ch, human) = run(&dir, &["status"]);
    assert_eq!(ch, 0, "{human}");
    assert!(
        human.contains("Subprojects: detected: 2 | adopted: 1 | pending: 1"),
        "human status must summarize adoption: {human}"
    );
}

#[test]
fn status_json_without_subprojects_reports_zero() {
    // A repo with no subprojects at all: zero counts, no section, no error.
    let dir = fixture_repo("node-single");
    let (c, out) = run(&dir, &["status", "--json"]);
    assert_eq!(c, 0, "status must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(
        v["subprojects"],
        serde_json::json!({"total": 0, "adopted": 0, "pending": 0})
    );
    assert_matches_schema(&v, "status-v1.json");
    let (ch, human) = run(&dir, &["status"]);
    assert_eq!(ch, 0, "{human}");
    assert!(
        !human.contains("Subprojects:"),
        "no manifest and no subprojects: no section expected: {human}"
    );

    // An unparseable manifest degrades to 0/0, never an error; the section
    // still appears because the file exists.
    let dir = repo_with(&[
        ("package.json", "{\"name\":\"plain\"}"),
        (".engineering/subprojects.yml", "schema: [unclosed"),
    ]);
    let (c, out) = run(&dir, &["status", "--json"]);
    assert_eq!(c, 0, "unparseable manifest must not fail status: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(
        v["subprojects"],
        serde_json::json!({"total": 0, "adopted": 0, "pending": 0})
    );
    let (ch, human) = run(&dir, &["status"]);
    assert_eq!(ch, 0, "{human}");
    assert!(
        human.contains("Subprojects: detected: 0 | adopted: 0 | pending: 0"),
        "an existing manifest shows the section even at zero: {human}"
    );
}

#[test]
fn impact_without_codegraph_degrades_to_scoped_text() {
    let dir = fixture_repo("monorepo-full");
    let bindir = git_only_path();
    let (c, out) = run_with_env(
        &dir,
        &[
            "search",
            "export const",
            "--in",
            "apps/web",
            "--impact",
            "--json",
        ],
        &bindir.to_string_lossy(),
        &[],
    );
    assert_eq!(c, 0, "impact must degrade, not fail: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(v["mode"], "impact");
    assert_eq!(v["backend"], "git grep");
    assert_eq!(v["scope"], "apps/web");
    assert!(
        v["note"]
            .as_str()
            .unwrap_or("")
            .contains("degraded to text results"),
        "the degradation note must survive scoping: {v:?}"
    );
    let paths = hits_paths(&v);
    assert!(
        !paths.is_empty(),
        "degraded results must be scoped, not empty"
    );
    assert!(
        paths.iter().all(|p| p.starts_with("apps/web/")),
        "degraded text results must stay inside the scope: {paths:?}"
    );
}

#[test]
fn impact_scope_sets_codegraph_working_directory() {
    let dir = fixture_repo("monorepo-full");
    let bindir = unique_base("codegraph-shim");
    let cwd_file = bindir.join("cwd.txt");
    let script = "#!/bin/sh\n\
        if [ \"$1\" = \"--version\" ]; then echo \"codegraph test-0.0.0\"; exit 0; fi\n\
        pwd > \"$CODEGRAPH_CWD_FILE\"\n\
        echo '{\"affected\":[]}'\n\
        exit 0\n";
    std::fs::write(bindir.join("codegraph"), script).expect("shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = bindir.join("codegraph");
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
    let path = format!(
        "{}:{}",
        bindir.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    );
    let cwd_path = cwd_file.to_string_lossy().to_string();
    let (c, out) = run_with_env(
        &dir,
        &["search", "web", "--in", "apps/web", "--impact", "--json"],
        &path,
        &[("CODEGRAPH_CWD_FILE", cwd_path.as_str())],
    );
    assert_eq!(c, 0, "impact with a working graph must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(v["mode"], "impact");
    assert_eq!(v["backend"], "codegraph");
    assert_eq!(v["scope"], "apps/web");
    let recorded = std::fs::read_to_string(&cwd_file).expect("shim must record its cwd");
    let recorded = PathBuf::from(recorded.trim());
    assert_eq!(
        std::fs::canonicalize(&recorded).expect("canonical recorded cwd"),
        std::fs::canonicalize(dir.join("apps/web")).expect("canonical scope"),
        "codegraph must run from the scoped directory"
    );
}
