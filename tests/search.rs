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
        "aicontext-search-{name}-{}-{n}-{:?}",
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

fn ast_grep_present() -> bool {
    Command::new("ast-grep")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn literal_search_json_contract() {
    let dir = fixture_repo("node-single");
    let (c, out) = run(&dir, ["search", "hello", "--json"]);
    assert_eq!(c, 0, "search must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("search --json must be JSON");
    assert_eq!(v["schema"], "aicontext/search/v1");
    assert_eq!(v["query"], "hello");
    assert_eq!(v["mode"], "text");
    let hits = v.get("hits").and_then(|x| x.as_array()).unwrap();
    assert!(
        hits.iter()
            .any(|h| h["path"].as_str().unwrap().contains("index.js")),
        "must find the fixture content, got {hits:?}"
    );
}

#[test]
fn search_hits_knowledge_first() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    // "Package manager" is written into the generated PROJECT_STATE block.
    let (cs, out) = run(&dir, ["search", "Package manager", "--json"]);
    assert_eq!(cs, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let knowledge = v.get("knowledge").and_then(|x| x.as_array()).unwrap();
    assert!(
        knowledge
            .iter()
            .any(|k| k["file"].as_str().unwrap().contains("PROJECT_STATE")),
        "knowledge hits must lead, got {knowledge:?}"
    );
}

#[test]
fn search_no_matches_is_success_with_empty_hits() {
    let dir = fixture_repo("rust-single");
    let (c, out) = run(&dir, ["search", "zzz-no-such-string", "--json"]);
    assert_eq!(c, 0, "no matches is not a failure: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["hits"].as_array().unwrap().is_empty());
}

#[test]
fn structural_search_uses_ast_grep() {
    if ast_grep_present() == false {
        // Without the binary the command must fail closed with exit 3.
        let dir = fixture_repo("node-single");
        let (c, _) = run(&dir, ["search", "console.log($A)", "--structure"]);
        assert_eq!(c, 3);
        return;
    }
    let dir = fixture_repo("node-single");
    let (c, out) = run(&dir, ["search", "console.log($A)", "--structure", "--json"]);
    assert_eq!(c, 0, "structural search must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["mode"], "structure");
    assert_eq!(v["backend"], "ast-grep");
    let hits = v.get("hits").and_then(|x| x.as_array()).unwrap();
    assert!(
        hits.iter()
            .any(|h| h["path"].as_str().unwrap().contains("index.js")),
        "ast-grep must match index.js, got {hits:?}"
    );
}

#[test]
fn impact_degrades_without_codegraph() {
    let dir = fixture_repo("node-single");
    let (c, out) = run(&dir, ["search", "hello", "--impact", "--json"]);
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["mode"], "impact");
    // No codegraph in P0: either degraded text results with a note,
    // or codegraph present (then text hits still supplement).
    assert!(v.get("note").is_some() || v["hits"].as_array().is_some());
}

fn tgrep_present() -> bool {
    Command::new("tgrep")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn literal_search_prefers_verified_tgrep_backend() {
    // Without tgrep the router degrades to rg/git grep (covered elsewhere).
    if tgrep_present() == false {
        return;
    }
    let dir = fixture_repo("node-single");
    let (c, out) = run(&dir, ["search", "hello", "--json"]);
    assert_eq!(c, 0, "search must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["backend"], "tgrep");
    let hits = v.get("hits").and_then(|x| x.as_array()).unwrap();
    assert!(
        hits.iter()
            .any(|h| h["path"].as_str().unwrap().contains("index.js")),
        "tgrep must find index.js, got {hits:?}"
    );
}
