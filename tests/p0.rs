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
    // Unique per call: parallel tests may use the same fixture name.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-p0-{name}-{}-{n}-{:?}",
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
fn init_is_idempotent_and_sync_is_zero_diff() {
    let dir = fixture_repo("node-single");
    let (c1, _) = run(&dir, ["init", "--non-interactive"]);
    // init returns the check code; node fixture has package.json so check passes.
    assert_eq!(c1, 0, "init should pass check on node fixture");
    assert!(dir.join(".engineering/aicontext.toml").exists());
    assert!(dir.join(".engineering/PROJECT_STATE.md").exists());
    assert!(dir.join(".engineering/PATTERNS.md").exists());
    assert!(dir.join(".engineering/consistency.yml").exists());

    let state_before = std::fs::read_to_string(dir.join(".engineering/PROJECT_STATE.md")).unwrap();
    let (c2, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c2, 0);
    let state_after = std::fs::read_to_string(dir.join(".engineering/PROJECT_STATE.md")).unwrap();
    assert_eq!(state_before, state_after, "double init must be zero diff");

    let (cs, _) = run(&dir, ["sync"]);
    assert_eq!(cs, 0);
    let (cc, _) = run(&dir, ["sync", "--check"]);
    assert_eq!(cc, 0, "sync --check must pass right after sync");
}

#[test]
fn sync_preserves_curated_block() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let marker = "CURATED-SENTINEL-123";
    let path = dir.join(".engineering/PROJECT_STATE.md");
    let text = std::fs::read_to_string(&path).unwrap();
    let patched = text.replace("TBD — describe", &format!("{marker} describe"));
    std::fs::write(&path, patched).unwrap();
    let (cs, _) = run(&dir, ["sync"]);
    assert_eq!(cs, 0);
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains(marker), "sync must preserve curated content");
}

#[test]
fn empty_commit_keeps_state_fresh() {
    // Regression: the generated block pins `Last synchronized commit`, so any
    // new commit — even one that changes no scanned facts — flipped freshness
    // to stale, and `check` could never pass on a committed tree. The commit
    // pointer is provenance, not drift: only scanned facts count.
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let (cs, _) = run(&dir, ["sync"]);
    assert_eq!(cs, 0);
    // An empty commit moves HEAD without touching any scanned fact: the tree
    // (tracked set, contents) is identical, so freshness must hold.
    git(&dir, ["commit", "-qm", "empty", "--allow-empty"]);
    let (cc, out) = run(&dir, ["sync", "--check"]);
    assert_eq!(cc, 0, "content-less commit must stay fresh: {out}");
    let (ck, cout) = run(&dir, ["check", "--json"]);
    assert_eq!(ck, 0, "check must pass on fresh tree: {cout}");
    let v: serde_json::Value = serde_json::from_str(&cout).expect("check --json must be JSON");
    let fresh = v
        .get("findings")
        .and_then(|f| f.as_array())
        .and_then(|fs| {
            fs.iter()
                .find(|f| f.get("name").and_then(|n| n.as_str()) == Some("freshness"))
        })
        .expect("check must include a freshness finding");
    assert_eq!(fresh.get("passed").and_then(|p| p.as_bool()), Some(true));
}

#[test]
fn sync_converges_when_block_grows() {
    // Regression: the scan measured `.engineering/` itself (md/toml/yaml count
    // as source), so whenever sync grew the generated block (e.g. a new
    // `Important paths` line), the rewritten state file added a LOC line and
    // the next check was stale by construction. Tool bookkeeping is not
    // product code: sync must converge.
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let (cs, _) = run(&dir, ["sync"]);
    assert_eq!(cs, 0);
    // A new product doc grows the generated block by one `Important paths` line.
    std::fs::write(dir.join("CHANGELOG.md"), "# Changelog\n").unwrap();
    git(&dir, ["add", "-A"]);
    git(&dir, ["commit", "-qm", "docs"]);
    let (cs2, _) = run(&dir, ["sync"]);
    assert_eq!(cs2, 0);
    let (cc, out) = run(&dir, ["sync", "--check"]);
    assert_eq!(cc, 0, "sync must converge after growing the block: {out}");
}

#[test]
fn check_detects_drift_after_source_change() {
    let dir = fixture_repo("rust-single");
    // rust fixture has no package.json; default version source is missing,
    // so init's trailing check may fail — init itself must still generate files.
    let _ = run(&dir, ["init", "--non-interactive"]);
    assert!(dir.join(".engineering/aicontext.toml").exists());
    let (cs0, _) = run(&dir, ["sync"]);
    assert_eq!(cs0, 0);
    // A new source file changes scanned facts (file count / LOC): genuine drift.
    std::fs::write(dir.join("src/extra.rs"), "pub fn extra() {}\n").unwrap();
    git(&dir, ["add", "-A"]);
    git(&dir, ["commit", "-qm", "second"]);
    let (cc, out) = run(&dir, ["sync", "--check"]);
    assert_eq!(cc, 1, "stale state must fail sync --check: {out}");
    let (cs, _) = run(&dir, ["sync"]);
    assert_eq!(cs, 0);
    let (cc2, _) = run(&dir, ["sync", "--check"]);
    assert_eq!(cc2, 0);
}

#[test]
fn scan_json_contract() {
    let dir = fixture_repo("bun-monorepo");
    let (c, out) = run(&dir, ["scan", "--json"]);
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).expect("scan --json must be JSON");
    assert_eq!(
        v.get("schema").and_then(|s| s.as_str()),
        Some("aicontext/scan/v2")
    );
    assert!(
        v.get("git")
            .and_then(|g| g.get("tracked_files"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0)
            >= 3
    );
    let managers: Vec<&str> = v
        .get("packages")
        .and_then(|p| p.as_array())
        .map(|ps| {
            ps.iter()
                .filter_map(|p| p.get("manager").and_then(|m| m.as_str()))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        managers.contains(&"npm"),
        "bun-monorepo fixture must detect npm manifest, got {managers:?}"
    );
    assert!(v
        .get("complexity")
        .and_then(|c| c.get("profile"))
        .and_then(|p| p.as_str())
        .is_some());
}

#[test]
fn check_json_contract() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let (cc, out) = run(&dir, ["check", "--json"]);
    assert_eq!(cc, 0);
    let v: serde_json::Value = serde_json::from_str(&out).expect("check --json must be JSON");
    assert_eq!(v["schema"], "aicontext/check/v1");
    assert_eq!(v["passed"], true);
    assert!(!v["findings"].as_array().unwrap().is_empty());
}
