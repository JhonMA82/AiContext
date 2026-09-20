//! Lifecycle gates of `check` for the monorepo router
//! (`subprojects routing` finding): with ≥2 detected subprojects the root
//! `AGENTS.md` must carry the marked routing block, manifest entries must
//! reconcile with detection in both directions (missing entries fail,
//! stale entries fail reusing `stale_mentions` for the docs still naming
//! them), `adopted` without a real nested context
//! (`<path>/.engineering/aicontext.toml`) fails closed, and `pending`
//! stays advisory. Shape problems belong to the `subprojects` finding.
//! All fixtures are copied into temp repositories (the committed fixtures
//! are read-only); nothing here touches the network.

use std::path::{Path, PathBuf};
use std::process::Command;

const ROUTING_START: &str = "<!-- aicontext:routing:start -->";
const ROUTING_END: &str = "<!-- aicontext:routing:end -->";

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
        "aicontext-gates-{tag}-{}-{n}-{:?}",
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

/// First `init` on a monorepo reports stale by design (creating `AGENTS.md`
/// adds a scanned doc); `sync` then converges to a green tree.
fn init_converged(dir: &Path) -> String {
    let (_, out) = run(dir, ["init", "--non-interactive"]);
    let (cs, sout) = run(dir, ["sync"]);
    assert_eq!(cs, 0, "sync must converge after init: {sout}");
    out
}

/// The `subprojects routing` finding of `check --json`, looked up by name.
fn routing_finding(dir: &Path) -> (i32, serde_json::Value) {
    let (code, out) = run(dir, ["check", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out)
        .unwrap_or_else(|e| panic!("check --json must be JSON ({e}): {out}"));
    let found = v["findings"]
        .as_array()
        .expect("findings must be an array")
        .iter()
        .find(|f| f["name"] == "subprojects routing")
        .unwrap_or_else(|| panic!("subprojects routing finding must exist: {v:?}"))
        .clone();
    (code, found)
}

fn manifest_text(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(".engineering/subprojects.yml")).expect("manifest")
}

fn write_manifest(dir: &Path, text: &str) {
    std::fs::write(dir.join(".engineering/subprojects.yml"), text).expect("write manifest");
}

#[test]
fn converged_monorepo_passes_routing_gates() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    let (code, found) = routing_finding(&dir);
    assert_eq!(code, 0, "converged tree must pass check: {found:?}");
    assert_eq!(found["passed"], true);
    let detail = found["detail"].as_str().unwrap_or("");
    assert!(detail.contains("routing block present"), "{detail}");
    assert!(detail.contains("pending (advisory)"), "{detail}");
}

#[test]
fn missing_routing_block_fails_with_init_remediation() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    // Remove the marked block (not the whole file): the gate reads markers,
    // so a legacy-looking file must still fail.
    let agents = std::fs::read_to_string(dir.join("AGENTS.md")).expect("agents");
    let s = agents.find(ROUTING_START).expect("routing block");
    let e = agents.find(ROUTING_END).expect("routing end") + ROUTING_END.len();
    let stripped = format!("{}{}", &agents[..s], &agents[e..]);
    assert!(!stripped.contains(ROUTING_START));
    std::fs::write(dir.join("AGENTS.md"), &stripped).expect("write agents");
    let (code, found) = routing_finding(&dir);
    assert_eq!(code, 1, "missing routing block must fail check");
    assert_eq!(found["passed"], false);
    assert!(
        found["detail"]
            .as_str()
            .unwrap_or("")
            .contains("aicontext init"),
        "{found:?}"
    );
}

#[test]
fn undetected_entry_addition_fails_as_missing() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    // A new real subproject the manifest does not declare yet.
    std::fs::create_dir_all(dir.join("apps/extra")).expect("mkdir");
    std::fs::write(
        dir.join("apps/extra/package.json"),
        "{\"name\":\"@t/extra\"}",
    )
    .expect("write");
    let (code, found) = routing_finding(&dir);
    assert_eq!(code, 1, "undeclared subproject must fail check");
    assert_eq!(found["passed"], false);
    let detail = found["detail"].as_str().unwrap_or("");
    assert!(detail.contains("apps/extra"), "{detail}");
    assert!(detail.contains("status: pending"), "{detail}");
}

#[test]
fn stale_manifest_entry_fails() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    let ghost = manifest_text(&dir) + "  apps/ghost:\n    purpose: \"\"\n    status: pending\n";
    write_manifest(&dir, &ghost);
    let (code, found) = routing_finding(&dir);
    assert_eq!(code, 1, "stale entry must fail check");
    assert_eq!(found["passed"], false);
    assert!(
        found["detail"]
            .as_str()
            .unwrap_or("")
            .contains("apps/ghost"),
        "{found:?}"
    );
}

#[test]
fn adopted_without_nested_context_fails_closed() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    let adopted = manifest_text(&dir).replace(
        "  apps/web:\n    purpose: \"\"\n    status: pending",
        "  apps/web:\n    purpose: Web app\n    status: adopted",
    );
    write_manifest(&dir, &adopted);
    let (code, found) = routing_finding(&dir);
    assert_eq!(code, 1, "hollow adoption must fail closed");
    assert_eq!(found["passed"], false);
    let detail = found["detail"].as_str().unwrap_or("");
    assert!(detail.contains("apps/web"), "{detail}");
    assert!(detail.contains("aicontext.toml"), "{detail}");
}

#[test]
fn adopted_with_nested_context_passes() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    let adopted = manifest_text(&dir).replace(
        "  apps/web:\n    purpose: \"\"\n    status: pending",
        "  apps/web:\n    purpose: Web app\n    status: adopted",
    );
    write_manifest(&dir, &adopted);
    // A real nested context: what a nested `init` leaves behind (WU5
    // automates this; the gate only reads the marker file).
    std::fs::create_dir_all(dir.join("apps/web/.engineering")).expect("mkdir");
    std::fs::write(
        dir.join("apps/web/.engineering/aicontext.toml"),
        "schema = \"aicontext/v1\"\n",
    )
    .expect("write");
    let (cs, sout) = run(&dir, ["sync"]);
    assert_eq!(cs, 0, "sync must converge: {sout}");
    let (code, found) = routing_finding(&dir);
    assert_eq!(code, 0, "grounded adoption must pass: {found:?}");
    assert_eq!(found["passed"], true);
    assert!(
        found["detail"]
            .as_str()
            .unwrap_or("")
            .contains("adopted: 1"),
        "{found:?}"
    );
}

#[test]
fn pending_entries_are_advisory_only() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    let (code, found) = routing_finding(&dir);
    assert_eq!(code, 0, "all-pending must pass check: {found:?}");
    assert_eq!(found["passed"], true);
}

#[test]
fn single_project_has_no_routing_gates() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0, "init must pass on a single project");
    let (code, found) = routing_finding(&dir);
    assert_eq!(code, 0);
    assert_eq!(found["passed"], true);
    assert!(
        found["detail"]
            .as_str()
            .unwrap_or("")
            .contains("fewer than 2"),
        "{found:?}"
    );
}

#[test]
fn absent_manifest_with_subprojects_fails_with_seed_remediation() {
    let dir = fixture_repo("monorepo-full");
    init_converged(&dir);
    std::fs::remove_file(dir.join(".engineering/subprojects.yml")).expect("remove");
    let (code, found) = routing_finding(&dir);
    assert_eq!(code, 1, "absent manifest with subprojects must fail");
    assert_eq!(found["passed"], false);
    assert!(
        found["detail"]
            .as_str()
            .unwrap_or("")
            .contains("aicontext init"),
        "{found:?}"
    );
}
