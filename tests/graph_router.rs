//! Offline graph-router tests. Fake `codebase-memory-mcp` and `codegraph`
//! executables live on a restricted PATH (only `git` is real), record every
//! invocation and answer from env fixtures, so routing order, degradation,
//! scope and bypass are proven without network, without real indexes and
//! without the machine's own graph tools leaking in.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
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

fn unique_base(prefix: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-graph-{prefix}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    std::fs::create_dir_all(&base).expect("mkdir");
    base
}

/// Fixture copy at `dst`, git-initialized so root resolution works.
fn fixture_at(fixture: &str, dst: &Path) -> PathBuf {
    if dst.exists() {
        std::fs::remove_dir_all(dst).expect("clean");
    }
    std::fs::create_dir_all(dst).expect("mkdir");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    copy_dir(&manifest.join("fixtures").join(fixture), dst);
    git(dst, ["init", "-q"]);
    git(dst, ["add", "-A"]);
    git(dst, ["commit", "-qm", "fixture"]);
    dst.to_path_buf()
}

fn fixture_repo(name: &str) -> PathBuf {
    fixture_at(name, &unique_base(name))
}

/// Fake codebase-memory-mcp: logs `$*`, answers from `CBM_*` env fixtures.
/// The tool name is scanned out of the args, exactly like the real CLI verb.
const CBM_SHIM: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CBM_LOG"
case "$1" in
  --version) echo "codebase-memory-mcp 0.11.0-test"; exit 0 ;;
esac
if [ -n "$CBM_HANG" ]; then exec __SLEEP__ 60; fi
tool=""
for a in "$@"; do
  case "$a" in
    list_projects|index_status|check_index_coverage|search_graph|trace_path|index_repository) tool="$a"; break ;;
  esac
done
if [ "$CBM_FAIL" = "all" ] || [ "$CBM_FAIL" = "$tool" ]; then exit 1; fi
case "$tool" in
  list_projects) printf '%s' "$CBM_PROJECTS_JSON" ;;
  index_status) printf '%s' "$CBM_INDEX_STATUS_JSON" ;;
  check_index_coverage) printf '%s' "$CBM_COVERAGE_JSON" ;;
  search_graph)
    case " $* " in
      *" --qn-pattern "*) printf '%s' "$CBM_GRAPH_QN_JSON" ;;
      *) printf '%s' "$CBM_GRAPH_NAME_JSON" ;;
    esac ;;
  trace_path) printf '%s' "$CBM_TRACE_JSON" ;;
  *) exit 1 ;;
esac
exit 0
"#;

/// Fake codegraph: logs `$*` and answers `impact` from `CODEGRAPH_IMPACT_JSON`.
const CODEGRAPH_SHIM: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CODEGRAPH_LOG"
case "$1" in
  --version) echo "codegraph 1.5.0-test"; exit 0 ;;
esac
if [ -n "$CODEGRAPH_HANG" ]; then exec __SLEEP__ 60; fi
if [ -n "$CODEGRAPH_FAIL" ]; then exit 1; fi
printf '%s' "$CODEGRAPH_IMPACT_JSON"
exit 0
"#;

/// A restricted PATH holding `git` plus the requested shims, with an isolated
/// HOME so the managed-tools registry of the real user can never interfere.
struct Shims {
    dir: PathBuf,
    cbm_log: PathBuf,
    codegraph_log: PathBuf,
}

fn locate(program: &str) -> String {
    let out = Command::new("sh")
        .args(["-c", &format!("command -v {program}")])
        .output()
        .expect(&format!("locate {program}"));
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!path.is_empty(), "{program} must exist to build shims");
    path
}

fn write_shim(path: PathBuf, body: &str) {
    std::fs::write(&path, body).expect("shim");
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&path).expect("meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("chmod");
}

fn shims(prefix: &str, cbm: bool, codegraph: bool) -> Shims {
    let dir = unique_base(prefix);
    std::os::unix::fs::symlink(locate("git"), dir.join("git")).expect("link git");
    if cbm || codegraph {
        // `sleep` is not a shell builtin: absolute path keeps the hang shim
        // working on the restricted PATH.
        let sleep = locate("sleep");
        if cbm {
            write_shim(
                dir.join("codebase-memory-mcp"),
                &CBM_SHIM.replace("__SLEEP__", &sleep),
            );
        }
        if codegraph {
            write_shim(
                dir.join("codegraph"),
                &CODEGRAPH_SHIM.replace("__SLEEP__", &sleep),
            );
        }
    }
    std::fs::create_dir_all(dir.join("home")).expect("home");
    Shims {
        cbm_log: dir.join("cbm.log"),
        codegraph_log: dir.join("codegraph.log"),
        dir,
    }
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Out {
    fn all(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
    fn json(&self) -> Value {
        let start = self
            .stdout
            .find('{')
            .unwrap_or_else(|| panic!("JSON output expected: {}", self.all()));
        serde_json::from_str(&self.stdout[start..]).expect("valid JSON")
    }
}

type Env = (&'static str, String);

fn run(dir: &Path, sh: &Shims, args: &[&str], envs: &[Env]) -> Out {
    let mut cmd = Command::new(bin());
    cmd.args(args)
        .current_dir(dir)
        .env("PATH", sh.dir.to_string_lossy().to_string())
        .env("HOME", sh.dir.join("home").to_string_lossy().to_string())
        .env("CBM_LOG", &sh.cbm_log)
        .env("CODEGRAPH_LOG", &sh.codegraph_log)
        .env("CBM_PROJECTS_JSON", "")
        .env("CBM_INDEX_STATUS_JSON", "")
        .env("CBM_COVERAGE_JSON", "")
        .env("CBM_GRAPH_NAME_JSON", "")
        .env("CBM_GRAPH_QN_JSON", "")
        .env("CBM_TRACE_JSON", "");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run aicontext");
    Out {
        code: out.status.code().unwrap_or(255),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn cbm_log(sh: &Shims) -> String {
    std::fs::read_to_string(&sh.cbm_log).unwrap_or_default()
}

fn codegraph_log(sh: &Shims) -> String {
    std::fs::read_to_string(&sh.codegraph_log).unwrap_or_default()
}

/// Healthy CBM answers for `root`: exact project match, ready index, a
/// `alpha` symbol in `scripts/`, its callee `beta` next to it and its caller
/// `gamma` in `other/`.
fn healthy_cbm(root: &Path) -> Vec<Env> {
    let root = root.to_string_lossy().to_string();
    vec![
        (
            "CBM_PROJECTS_JSON",
            json!({"projects":[{"name":"fixture-project","root_path":root,"branch":"main"}],
                   "total":1,"offset":0,"limit":50,"returned":1,"has_more":false})
            .to_string(),
        ),
        (
            "CBM_INDEX_STATUS_JSON",
            json!({"project":"fixture-project","nodes":3,"edges":2,"status":"ready",
                   "root_path":root,"indexed_at":"2026-09-22T00:00:00Z"})
            .to_string(),
        ),
        (
            "CBM_COVERAGE_JSON",
            json!({"project":"fixture-project","signal":"best_effort",
                   "paths":[{"path":"scripts","status":"no_recorded_issue","freshness":"not_tracked"}]})
            .to_string(),
        ),
        (
            "CBM_GRAPH_NAME_JSON",
            json!({"cols":["name","label","lines","in","out"],
                   "groups":[{"qn_prefix":"proj.src","file":"scripts/alpha.js",
                              "rows":[["alpha","Function","10-20",1,2]]}],
                   "total":1,"returned":1,"has_more":false,"truncated":false})
            .to_string(),
        ),
        (
            "CBM_GRAPH_QN_JSON",
            json!({"cols":["name","label","lines","in","out"],
                   "groups":[{"qn_prefix":"proj.src","file":"scripts/alpha.js",
                              "rows":[["beta","Function","30-40",2,0]]},
                             {"qn_prefix":"proj.other","file":"other/gamma.js",
                              "rows":[["gamma","Function","5-9",0,1]]}],
                   "total":2,"returned":2,"has_more":false,"truncated":false})
            .to_string(),
        ),
        (
            "CBM_TRACE_JSON",
            json!({"function":"proj.src.alpha","direction":"both",
                   "callees":{"cols":["name","hop"],"groups":[{"qn_prefix":"proj.src","rows":[["beta",1]]}]},
                   "callers":{"cols":["name","hop"],"groups":[{"qn_prefix":"proj.other","rows":[["gamma",1]]}]},
                   "truncated":false})
            .to_string(),
        ),
    ]
}

/// CodeGraph impact answer naming one affected file.
fn codegraph_answer() -> Vec<Env> {
    vec![(
        "CODEGRAPH_IMPACT_JSON",
        json!({"affected":[{"filePath":"graph/affected.js","startLine":3,"name":"alpha","kind":"function"}]})
            .to_string(),
    )]
}

/// Seeds `.engineering/` in a fixture. `init` may finish with a non-zero
/// trailing check; the manifest is what the later steps need.
fn init_context(dir: &Path) {
    let sh = shims("init", false, false);
    let out = run(dir, &sh, &["init", "--non-interactive"], &[]);
    assert!(
        dir.join(".engineering").join("aicontext.toml").exists(),
        "init must write the manifest: {}",
        out.all()
    );
}

/// Minimal contract check over `schemas/<file>`: `required`, `const`, `enum`
/// and `type`, mirroring the subprojects contract test. Not a general
/// JSON-schema implementation — it locks the fields this work emits.
fn assert_matches_schema(value: &Value, schema: &Value, prefix: &str) {
    for req in schema["required"].as_array().into_iter().flatten() {
        let key = req.as_str().expect("required key");
        assert!(
            value.get(key).is_some(),
            "{prefix}: missing required field {key}"
        );
    }
    if let Some(c) = schema.get("const") {
        assert_eq!(value, c, "{prefix}: const mismatch");
    }
    if let Some(Value::Array(allowed)) = schema.get("enum") {
        assert!(
            allowed.contains(value),
            "{prefix}: {value} is not in the enum"
        );
    }
    if let Some(t) = schema.get("type").and_then(|t| t.as_str()) {
        let ok = match t {
            "object" => value.is_object(),
            "string" => value.is_string(),
            "integer" => value.is_i64(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            _ => true,
        };
        assert!(ok, "{prefix}: {value} is not a {t}");
    }
    for (key, sub) in schema["properties"].as_object().into_iter().flatten() {
        if let Some(v) = value.get(key) {
            assert_matches_schema(v, sub, &format!("{prefix}.{key}"));
        }
    }
    if let (Some(item_schema), Some(values)) = (schema.get("items"), value.as_array()) {
        for (i, item) in values.iter().enumerate() {
            assert_matches_schema(item, item_schema, &format!("{prefix}[{i}]"));
        }
    }
}

fn search_schema() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("schemas")
        .join("search-v1.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("read search-v1.json"))
        .expect("search-v1.json must be JSON")
}

// 1. `tools status --json` contains codebase-memory-mcp.
#[test]
fn tools_status_reports_codebase_memory_mcp() {
    let dir = fixture_repo("node-single");
    let sh = shims("status-cbm", true, true);
    let out = run(&dir, &sh, &["tools", "status", "--json"], &[]);
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    let tools = v["tools"].as_array().expect("tools array");
    assert!(
        tools.iter().any(|t| t["name"] == "codebase-memory-mcp"),
        "tools status must list codebase-memory-mcp: {v}"
    );
}

// 2. CodeGraph keeps appearing.
#[test]
fn tools_status_still_reports_codegraph() {
    let dir = fixture_repo("node-single");
    let sh = shims("status-cg", true, true);
    let out = run(&dir, &sh, &["tools", "status", "--json"], &[]);
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    let tools = v["tools"].as_array().expect("tools array");
    assert!(
        tools.iter().any(|t| t["name"] == "codegraph"),
        "tools status must keep listing codegraph: {v}"
    );
}

// 3. `search "foo"` never runs CBM nor CodeGraph.
#[test]
fn text_search_never_touches_graph_backends() {
    let dir = fixture_repo("node-single");
    let sh = shims("bypass-text", true, true);
    let out = run(
        &dir,
        &sh,
        &["search", "hello", "--json"],
        &healthy_cbm(&dir),
    );
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(v["mode"], "text");
    assert!(v["hits"].as_array().expect("hits").len() >= 1);
    assert!(
        cbm_log(&sh).is_empty(),
        "text search called CBM: {}",
        cbm_log(&sh)
    );
    assert!(
        codegraph_log(&sh).is_empty(),
        "text search called codegraph: {}",
        codegraph_log(&sh)
    );
}

// 4. `search --structure` never runs CBM nor CodeGraph.
#[test]
fn structure_search_never_touches_graph_backends() {
    let dir = fixture_repo("node-single");
    let sh = shims("bypass-structure", true, true);
    let out = run(
        &dir,
        &sh,
        &["search", "console.log($A)", "--structure", "--json"],
        &healthy_cbm(&dir),
    );
    // Without ast-grep on the restricted PATH this fails closed (exit 3);
    // either way the graph backends must stay untouched.
    assert!(
        out.code == 0 || out.code == 3,
        "unexpected exit {}: {}",
        out.code,
        out.all()
    );
    assert!(
        cbm_log(&sh).is_empty(),
        "structure search called CBM: {}",
        cbm_log(&sh)
    );
    assert!(
        codegraph_log(&sh).is_empty(),
        "structure search called codegraph: {}",
        codegraph_log(&sh)
    );
}

// 5. `search --impact` with a healthy CBM uses CBM.
#[test]
fn impact_prefers_codebase_memory_when_healthy() {
    let dir = fixture_repo("node-single");
    let sh = shims("impact-cbm", true, true);
    let out = run(
        &dir,
        &sh,
        &["search", "alpha", "--impact", "--json"],
        &healthy_cbm(&dir),
    );
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(v["mode"], "impact");
    assert_eq!(v["backend"], "codebase-memory-mcp", "{v}");
    assert!(
        v.get("note").is_none(),
        "healthy graph answer has no note: {v}"
    );
    let hits: Vec<(String, u64, String)> = v["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .map(|h| {
            (
                h["path"].as_str().unwrap().to_string(),
                h["line"].as_u64().unwrap(),
                h["text"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        hits,
        vec![
            (
                "other/gamma.js".to_string(),
                5,
                "gamma (Function, caller)".to_string()
            ),
            (
                "scripts/alpha.js".to_string(),
                10,
                "alpha (Function)".to_string()
            ),
            (
                "scripts/alpha.js".to_string(),
                30,
                "beta (Function, callee)".to_string()
            ),
        ],
        "graph hits must name the symbol and its direct relations"
    );
}

// 6. With a healthy CBM, CodeGraph never runs.
#[test]
fn healthy_codebase_memory_skips_codegraph() {
    let dir = fixture_repo("node-single");
    let sh = shims("skip-cg", true, true);
    let out = run(
        &dir,
        &sh,
        &["search", "alpha", "--impact", "--json"],
        &healthy_cbm(&dir),
    );
    assert_eq!(out.code, 0, "{}", out.all());
    assert_eq!(out.json()["backend"], "codebase-memory-mcp");
    assert!(
        codegraph_log(&sh).is_empty(),
        "a Ready CBM answer must stop the router, codegraph ran: {}",
        codegraph_log(&sh)
    );
}

// 7. CBM absent -> CodeGraph.
#[test]
fn missing_codebase_memory_falls_back_to_codegraph() {
    let dir = fixture_repo("node-single");
    let sh = shims("fallback-cg", false, true);
    let out = run(
        &dir,
        &sh,
        &["search", "alpha", "--impact", "--json"],
        &codegraph_answer(),
    );
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(v["backend"], "codegraph", "{v}");
    let hits = v["hits"].as_array().expect("hits");
    assert_eq!(hits[0]["path"], "graph/affected.js");
    assert_eq!(hits[0]["line"], 3);
}

// 8. CBM present without an index of this exact root -> CodeGraph.
#[test]
fn unindexed_root_falls_back_to_codegraph() {
    let dir = fixture_repo("node-single");
    let sh = shims("no-index", true, true);
    let mut envs = healthy_cbm(&dir);
    let other = unique_base("other-root");
    envs[0] = (
        "CBM_PROJECTS_JSON",
        json!({"projects":[{"name":"other-project","root_path":other.to_string_lossy(),"branch":"main"}],
               "total":1,"returned":1,"has_more":false})
        .to_string(),
    );
    envs.extend(codegraph_answer());
    let out = run(&dir, &sh, &["search", "alpha", "--impact", "--json"], &envs);
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(v["backend"], "codegraph", "{v}");
    let hits = v["hits"].as_array().expect("hits");
    assert_eq!(hits[0]["path"], "graph/affected.js", "{v}");
    assert!(
        cbm_log(&sh).contains("list_projects"),
        "CBM must have been consulted first: {}",
        cbm_log(&sh)
    );
}

// 9. CBM wedged past its budget -> CodeGraph.
#[test]
fn codebase_memory_timeout_falls_back_to_codegraph() {
    let dir = fixture_repo("node-single");
    let sh = shims("timeout", true, true);
    let mut envs = healthy_cbm(&dir);
    envs.push(("CBM_HANG", "1".to_string()));
    // Small budget so the offline timeout test stays fast.
    envs.push(("AICONTEXT_GRAPH_BUDGET_SECS", "2".to_string()));
    envs.extend(codegraph_answer());
    let out = run(&dir, &sh, &["search", "alpha", "--impact", "--json"], &envs);
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(v["backend"], "codegraph", "a wedged CBM must not win: {v}");
    assert!(
        cbm_log(&sh).contains("list_projects"),
        "CBM must have been tried"
    );
}

// 10. CBM fails and CodeGraph fails -> literal fallback.
#[test]
fn both_graph_backends_failing_falls_back_to_text() {
    let dir = fixture_repo("node-single");
    let sh = shims("both-fail", true, true);
    let mut envs = healthy_cbm(&dir);
    envs.push(("CBM_FAIL", "all".to_string()));
    envs.push(("CODEGRAPH_FAIL", "1".to_string()));
    envs.extend(codegraph_answer());
    let out = run(&dir, &sh, &["search", "hello", "--impact", "--json"], &envs);
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert!(
        ["tgrep", "rg", "git grep"].contains(&v["backend"].as_str().unwrap_or("")),
        "text fallback must stay a text backend: {v}"
    );
    assert!(v["hits"].as_array().expect("hits").len() >= 1, "{v}");
    assert!(
        v["note"].as_str().unwrap_or("").contains("degraded"),
        "degradation must be announced: {v}"
    );
}

// 11. Neither backend installed -> current text degradation.
#[test]
fn impact_without_any_graph_backend_degrades_to_text() {
    let dir = fixture_repo("node-single");
    let sh = shims("none", false, false);
    let out = run(&dir, &sh, &["search", "hello", "--impact", "--json"], &[]);
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(v["mode"], "impact");
    assert!(
        ["tgrep", "rg", "git grep"].contains(&v["backend"].as_str().unwrap_or("")),
        "text fallback must stay a text backend: {v}"
    );
    let note = v["note"].as_str().expect("degraded search carries a note");
    assert!(note.contains("degraded"), "{note}");
    assert!(note.contains("codebase-memory-mcp"), "{note}");
    assert!(note.contains("codegraph"), "{note}");
}

// 12. An empty-but-valid graph answer is terminal: no second backend runs.
#[test]
fn empty_graph_result_is_terminal() {
    let dir = fixture_repo("node-single");
    std::fs::create_dir_all(dir.join("local")).expect("scope dir");
    let sh = shims("empty-valid", true, true);
    // Every graph node lives outside `local/`, so the scoped answer is empty.
    let mut envs = healthy_cbm(&dir);
    envs[2] = (
        "CBM_COVERAGE_JSON",
        json!({"project":"fixture-project","signal":"best_effort",
               "paths":[{"path":"local","status":"no_recorded_issue","freshness":"not_tracked"}]})
        .to_string(),
    );
    let out = run(
        &dir,
        &sh,
        &["search", "alpha", "--impact", "--in", "local", "--json"],
        &envs,
    );
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(v["backend"], "codebase-memory-mcp", "{v}");
    assert_eq!(v["hits"].as_array().expect("hits").len(), 0, "{v}");
    assert!(
        v.get("note").is_none(),
        "a valid empty answer has no note: {v}"
    );
    assert!(
        codegraph_log(&sh).is_empty(),
        "a valid empty graph answer must not trigger codegraph: {}",
        codegraph_log(&sh)
    );
}

// 13. `--in` keeps the scope: nothing outside it is reported.
#[test]
fn impact_scope_is_preserved() {
    let dir = fixture_repo("node-single");
    std::fs::create_dir_all(dir.join("scripts")).expect("scope dir");
    std::fs::create_dir_all(dir.join("other")).expect("scope dir");
    let sh = shims("scope", true, true);
    let out = run(
        &dir,
        &sh,
        &["search", "alpha", "--impact", "--in", "scripts", "--json"],
        &healthy_cbm(&dir),
    );
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(v["backend"], "codebase-memory-mcp", "{v}");
    assert_eq!(v["scope"], "scripts");
    let hits = v["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 2, "only in-scope nodes are reported: {v}");
    for h in hits {
        let path = h["path"].as_str().unwrap();
        assert!(
            path.starts_with("scripts/"),
            "scoped hit escaped the scope: {h}"
        );
    }
}

// 13b. A scope the index cannot demonstrate degrades instead of mislabelling
//      repo-wide results as scoped.
#[test]
fn uncovered_scope_degrades_to_next_backend() {
    let dir = fixture_repo("node-single");
    std::fs::create_dir_all(dir.join("scripts")).expect("scope dir");
    let sh = shims("uncovered", true, true);
    let mut envs = healthy_cbm(&dir);
    envs[2] = (
        "CBM_COVERAGE_JSON",
        json!({"project":"fixture-project","signal":"best_effort",
               "paths":[{"path":"scripts","status":"no_recorded_issue","freshness":"missing"}]})
        .to_string(),
    );
    envs.extend(codegraph_answer());
    let out = run(
        &dir,
        &sh,
        &["search", "alpha", "--impact", "--in", "scripts", "--json"],
        &envs,
    );
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(
        v["backend"], "codegraph",
        "uncovered scope must degrade: {v}"
    );
}

// 14. `--subproject` keeps its validation and its scope.
#[test]
fn subproject_validation_is_preserved() {
    let dir = fixture_repo("monorepo-full");
    let sh = shims("subproject", true, true);
    // Unknown subproject: rejected before any backend is invoked.
    let bad = run(
        &dir,
        &sh,
        &[
            "search",
            "alpha",
            "--impact",
            "--subproject",
            "nope",
            "--json",
        ],
        &healthy_cbm(&dir),
    );
    assert_eq!(bad.code, 5, "{}", bad.all());
    assert!(
        bad.all().contains("not a detected subproject"),
        "{}",
        bad.all()
    );
    assert!(
        cbm_log(&sh).is_empty(),
        "invalid scope must not reach CBM: {}",
        cbm_log(&sh)
    );

    // Detected subproject: served by the graph, scoped to the subproject.
    let mut envs = healthy_cbm(&dir);
    envs[2] = (
        "CBM_COVERAGE_JSON",
        json!({"project":"fixture-project","signal":"best_effort",
               "paths":[{"path":"apps/web","status":"no_recorded_issue","freshness":"not_tracked"}]})
        .to_string(),
    );
    envs[3] = (
        "CBM_GRAPH_NAME_JSON",
        json!({"cols":["name","label","lines","in","out"],
               "groups":[{"qn_prefix":"proj.web","file":"apps/web/src/alpha.js",
                          "rows":[["alpha","Function","10-20",1,0]]}],
               "total":1,"returned":1,"has_more":false,"truncated":false})
        .to_string(),
    );
    envs[4] = (
        "CBM_GRAPH_QN_JSON",
        json!({"cols":["name","label","lines","in","out"],"groups":[],
               "total":0,"returned":0,"has_more":false,"truncated":false})
        .to_string(),
    );
    envs[5] = (
        "CBM_TRACE_JSON",
        json!({"function":"proj.web.alpha","direction":"both",
               "callees":{"cols":["name","hop"],"groups":[]},
               "callers":{"cols":["name","hop"],"groups":[]},
               "truncated":false})
        .to_string(),
    );
    let good = run(
        &dir,
        &sh,
        &[
            "search",
            "alpha",
            "--impact",
            "--subproject",
            "apps/web",
            "--json",
        ],
        &envs,
    );
    assert_eq!(good.code, 0, "{}", good.all());
    let v = good.json();
    assert_eq!(v["backend"], "codebase-memory-mcp", "{v}");
    assert_eq!(v["subproject"], "apps/web");
    let hits = v["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 1, "{v}");
    assert_eq!(hits[0]["path"], "apps/web/src/alpha.js");
}

// 15. An index of another repository with the same basename is rejected.
#[test]
fn same_basename_project_elsewhere_is_rejected() {
    let base = unique_base("basename");
    let mine = fixture_at("node-single", &base.join("one").join("repo"));
    std::fs::create_dir_all(base.join("two").join("repo")).expect("decoy");
    let decoy = base.join("two").join("repo");
    let sh = shims("basename", true, true);
    let mut envs = healthy_cbm(&mine);
    envs[0] = (
        "CBM_PROJECTS_JSON",
        json!({"projects":[{"name":"decoy-project","root_path":decoy.to_string_lossy(),"branch":"main"}],
               "total":1,"returned":1,"has_more":false})
        .to_string(),
    );
    envs.extend(codegraph_answer());
    let out = run(
        &mine,
        &sh,
        &["search", "alpha", "--impact", "--json"],
        &envs,
    );
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(
        v["backend"], "codegraph",
        "same-basename index must never be accepted: {v}"
    );
    assert_eq!(
        v["hits"].as_array().expect("hits")[0]["path"],
        "graph/affected.js"
    );
}

// 16. `search` never calls `index_repository`.
#[test]
fn search_never_calls_index_repository() {
    let dir = fixture_repo("node-single");
    let sh = shims("no-index-repo", true, true);
    for args in [
        vec!["search", "hello", "--json"],
        vec!["search", "alpha", "--impact", "--json"],
    ] {
        let out = run(&dir, &sh, &args, &healthy_cbm(&dir));
        assert_eq!(out.code, 0, "{}", out.all());
    }
    let log = cbm_log(&sh);
    assert!(!log.is_empty(), "the healthy path must have talked to CBM");
    assert!(
        !log.contains("index_repository"),
        "search must never index as a side effect: {log}"
    );
}

// 17. `aicontext/search/v1` still validates (and stays v1).
#[test]
fn search_contract_v1_still_validates() {
    let schema = search_schema();
    assert_eq!(
        schema["$id"], "aicontext/search/v1",
        "the search contract must stay on v1"
    );
    let dir = fixture_repo("node-single");
    let sh = shims("contract", true, true);
    let graph = run(
        &dir,
        &sh,
        &["search", "alpha", "--impact", "--json"],
        &healthy_cbm(&dir),
    );
    assert_eq!(graph.code, 0, "{}", graph.all());
    assert_matches_schema(&graph.json(), &schema, "$");
    let text = run(
        &dir,
        &sh,
        &["search", "hello", "--json"],
        &healthy_cbm(&dir),
    );
    assert_eq!(text.code, 0, "{}", text.all());
    assert_matches_schema(&text.json(), &schema, "$");
}

// 18. `doctor` on a large repo: CBM / CodeGraph / none. Never blocking.
#[test]
fn doctor_large_repo_reports_graph_backend() {
    let dir = fixture_repo("node-single");
    init_context(&dir);
    // `repository.size` override keeps the fixture small while `doctor` sees
    // a large repository (the same path a real large repo takes).
    let manifest = dir.join(".engineering").join("aicontext.toml");
    let mut toml = std::fs::read_to_string(&manifest).expect("manifest");
    toml.push_str("\n[repository]\nsize = \"large\"\n");
    std::fs::write(&manifest, toml).expect("manifest");

    let cases = [(true, false), (false, true), (false, false)];
    for (cbm, codegraph) in cases {
        let sh = shims(&format!("doctor-{cbm}-{codegraph}"), cbm, codegraph);
        let out = run(&dir, &sh, &["doctor", "--json"], &healthy_cbm(&dir));
        assert_eq!(
            out.code,
            0,
            "doctor must never block on a missing graph: {}",
            out.all()
        );
        let v = out.json();
        let diag = v["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .find(|d| d["name"] == "graph backend")
            .unwrap_or_else(|| panic!("graph backend diagnostic missing: {v}"));
        assert_ne!(
            diag["status"], "fail",
            "graph absence must stay advisory: {diag}"
        );
        match (cbm, codegraph) {
            (true, _) => {
                assert_eq!(diag["status"], "ok", "{diag}");
                assert!(
                    diag["detail"]
                        .as_str()
                        .unwrap()
                        .contains("graph backend available"),
                    "{diag}"
                );
                assert!(
                    diag["detail"]
                        .as_str()
                        .unwrap()
                        .contains("codebase-memory-mcp"),
                    "{diag}"
                );
            }
            (false, true) => {
                assert_eq!(diag["status"], "ok", "{diag}");
                assert!(
                    diag["detail"]
                        .as_str()
                        .unwrap()
                        .contains("graph backend available"),
                    "{diag}"
                );
                assert!(
                    diag["detail"].as_str().unwrap().contains("codegraph"),
                    "{diag}"
                );
            }
            (false, false) => {
                assert_eq!(diag["status"], "warn", "{diag}");
                assert!(
                    diag["detail"]
                        .as_str()
                        .unwrap()
                        .contains("no graph backend"),
                    "{diag}"
                );
                assert_eq!(
                    diag["remediation"].as_str(),
                    Some("run `aicontext tools plan`"),
                    "{diag}"
                );
            }
        }
    }
}

// 19. Equivalent fixtures produce byte-stable output.
#[test]
fn impact_output_is_deterministic_across_equivalent_fixtures() {
    let first = fixture_repo("node-single");
    let second = fixture_repo("node-single");
    let mut answers = Vec::new();
    for dir in [&first, &second] {
        let sh = shims("deterministic", true, true);
        let out = run(
            dir,
            &sh,
            &["search", "alpha", "--impact", "--json"],
            &healthy_cbm(dir),
        );
        assert_eq!(out.code, 0, "{}", out.all());
        answers.push(out.json());
    }
    assert_eq!(
        answers[0]["hits"], answers[1]["hits"],
        "equivalent fixtures must answer identically"
    );
    assert_eq!(answers[0]["backend"], answers[1]["backend"]);
    // And the same fixture answers identically twice in a row.
    let sh = shims("deterministic-repeat", true, true);
    let a = run(
        &first,
        &sh,
        &["search", "alpha", "--impact", "--json"],
        &healthy_cbm(&first),
    );
    let b = run(
        &first,
        &sh,
        &["search", "alpha", "--impact", "--json"],
        &healthy_cbm(&first),
    );
    assert_eq!(
        a.json()["hits"],
        b.json()["hits"],
        "repeat run must be stable"
    );
}

// Extra: `mode = "off"` disables a graph backend without uninstalling it.
#[test]
fn config_off_disables_graph_backends() {
    let dir = fixture_repo("node-single");
    init_context(&dir);
    let manifest = dir.join(".engineering").join("aicontext.toml");
    let toml = std::fs::read_to_string(&manifest).expect("manifest");
    let toml = toml.replace(
        "[tools.codebase_memory]\nmode = \"auto\"",
        "[tools.codebase_memory]\nmode = \"off\"",
    );
    assert!(
        toml.contains("[tools.codebase_memory]\nmode = \"off\""),
        "manifest must declare the codebase_memory mode"
    );
    std::fs::write(&manifest, toml).expect("manifest");

    let sh = shims("off", true, true);
    let out = run(&dir, &sh, &["search", "alpha", "--impact", "--json"], &{
        let mut envs = healthy_cbm(&dir);
        envs.extend(codegraph_answer());
        envs
    });
    assert_eq!(out.code, 0, "{}", out.all());
    let v = out.json();
    assert_eq!(
        v["backend"], "codegraph",
        "disabled CBM must not be consulted: {v}"
    );
    assert!(
        cbm_log(&sh).is_empty(),
        "a disabled backend must never be invoked: {}",
        cbm_log(&sh)
    );
}
