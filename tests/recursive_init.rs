//! `init --recursive` and nested project resolution: the flag seeds one
//! nested `.engineering/` context per detected subproject (manifest with
//! the `[subproject]` parent pointer, generated state, patterns
//! placeholder, consistency stub) plus the marked context pointer in each
//! subproject's own `AGENTS.md` — never creating that file — and
//! `resolve_project_root()` makes every command operate on the nearest
//! manifest up the tree (nested context when inside one, git toplevel
//! otherwise). All fixtures are copied into temp repositories (the
//! committed fixtures are read-only); nothing here touches the network.

use std::path::{Path, PathBuf};
use std::process::Command;

const CONTEXT_START: &str = "<!-- aicontext:context:start -->";

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
}

fn run_in<const N: usize>(dir: &Path, args: [&str; N]) -> (i32, String) {
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
        "aicontext-recursive-{tag}-{}-{n}-{:?}",
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

/// A declared workspace without any `AGENTS.md`: root `package.json` with
/// workspaces plus two plain package subprojects.
fn synth_repo() -> PathBuf {
    let base = temp_base("synth");
    for (rel, content) in [
        (
            "package.json",
            "{\"name\":\"root\",\"workspaces\":[\"a\",\"b\"]}",
        ),
        ("a/package.json", "{\"name\":\"@t/a\"}"),
        ("b/package.json", "{\"name\":\"@t/b\"}"),
    ] {
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

fn nested_manifest(dir: &Path, sub: &str) -> String {
    std::fs::read_to_string(dir.join(sub).join(".engineering/aicontext.toml"))
        .unwrap_or_else(|_| panic!("nested manifest missing for {sub}"))
}

#[test]
fn plain_init_creates_no_nested_contexts() {
    let dir = fixture_repo("monorepo-full");
    let _ = run_in(&dir, ["init", "--non-interactive"]);
    for sub in ["apps/desktop", "apps/mobile", "apps/web", "services/api"] {
        assert!(
            !dir.join(sub).join(".engineering").exists(),
            "plain init must not create {sub}/.engineering"
        );
    }
    let root_manifest =
        std::fs::read_to_string(dir.join(".engineering/aicontext.toml")).expect("root manifest");
    assert!(
        !root_manifest.contains("[subproject]"),
        "root manifest is not nested"
    );
}

#[test]
fn recursive_seeds_nested_contexts_with_parent_pointers() {
    let dir = fixture_repo("monorepo-full");
    let (_, out) = run_in(&dir, ["init", "--non-interactive", "--recursive"]);
    for sub in ["apps/desktop", "apps/mobile", "apps/web", "services/api"] {
        assert!(
            out.contains(&format!("{sub}: nested context seeded")),
            "note per subproject: {out}"
        );
        let manifest = nested_manifest(&dir, sub);
        assert!(
            manifest.contains("[subproject]"),
            "{sub}: parent pointer section"
        );
        assert!(
            manifest.contains("parent = \"../..\""),
            "{sub}: parent climbs two levels"
        );
        for file in [
            ".engineering/PROJECT_STATE.md",
            ".engineering/PATTERNS.md",
            ".engineering/consistency.yml",
        ] {
            assert!(dir.join(sub).join(file).is_file(), "{sub}/{file} seeded");
        }
    }
    let web = nested_manifest(&dir, "apps/web");
    assert!(
        web.contains("source = \"package.json\""),
        "npm subproject version source"
    );
    let api = nested_manifest(&dir, "services/api");
    assert!(
        api.contains("source = \"Cargo.toml\""),
        "cargo subproject version source"
    );
}

#[test]
fn recursive_runs_are_byte_identical() {
    let dir = fixture_repo("monorepo-full");
    let _ = run_in(&dir, ["init", "--non-interactive", "--recursive"]);
    let subs = ["apps/desktop", "apps/mobile", "apps/web", "services/api"];
    let mut snapshot: Vec<(String, String)> = Vec::new();
    for sub in subs {
        for f in [
            "aicontext.toml",
            "PROJECT_STATE.md",
            "PATTERNS.md",
            "consistency.yml",
        ] {
            let rel = format!("{sub}/.engineering/{f}");
            let text = std::fs::read_to_string(dir.join(&rel)).expect("seeded file");
            snapshot.push((rel, text));
        }
    }
    let agents: Vec<(String, String)> = ["apps/desktop", "apps/mobile", "apps/web", "services/api"]
        .iter()
        .map(|sub| {
            let rel = format!("{sub}/AGENTS.md");
            (
                rel.clone(),
                std::fs::read_to_string(dir.join(&rel)).expect("agents"),
            )
        })
        .collect();
    let (_, out) = run_in(&dir, ["init", "--non-interactive", "--recursive"]);
    assert!(out.contains("left intact"), "rerun must not reseed: {out}");
    for (rel, before) in snapshot.iter().chain(agents.iter()) {
        let after = std::fs::read_to_string(dir.join(rel)).expect("file");
        assert_eq!(&after, before, "{rel} must be byte-identical across runs");
    }
}

#[test]
fn nested_agents_md_gets_context_pointer_only() {
    let dir = fixture_repo("monorepo-full");
    let before = std::fs::read_to_string(dir.join("apps/web/AGENTS.md")).expect("fixture agents");
    let _ = run_in(&dir, ["init", "--non-interactive", "--recursive"]);
    let after = std::fs::read_to_string(dir.join("apps/web/AGENTS.md")).expect("agents");
    assert!(after.contains(CONTEXT_START), "context pointer inserted");
    assert!(
        !after.contains("aicontext:routing"),
        "no routing block in a nested project"
    );
    for line in before.lines() {
        assert!(after.contains(line), "user bytes preserved: {line}");
    }
}

#[test]
fn agents_md_is_never_created_in_subprojects() {
    let dir = synth_repo();
    let _ = run_in(&dir, ["init", "--non-interactive", "--recursive"]);
    for sub in ["a", "b"] {
        assert!(
            dir.join(sub).join(".engineering/aicontext.toml").is_file(),
            "{sub} nested context seeded"
        );
        assert!(
            !dir.join(sub).join("AGENTS.md").exists(),
            "{sub}/AGENTS.md must never be created"
        );
    }
}

#[test]
fn commands_resolve_to_the_nested_root() {
    let dir = fixture_repo("monorepo-full");
    let _ = run_in(&dir, ["init", "--non-interactive", "--recursive"]);
    let web = dir.join("apps/web");
    let (cs, sout) = run_in(&web, ["status"]);
    assert_eq!(cs, 0, "{sout}");
    assert!(
        sout.contains(&format!("Repo: {}", web.to_string_lossy())),
        "status must report the nested root: {sout}"
    );
    let (cc, cout) = run_in(&web, ["scan", "--json"]);
    assert_eq!(cc, 0, "{cout}");
    let v: serde_json::Value = serde_json::from_str(&cout).expect("scan JSON");
    let root = v["git"]["root"].as_str().unwrap_or("");
    assert!(
        root.ends_with("apps/web"),
        "scan must be rooted at the nested context: {root}"
    );
}

#[test]
fn nested_contexts_pass_their_own_check() {
    let dir = fixture_repo("monorepo-full");
    let _ = run_in(&dir, ["init", "--non-interactive", "--recursive"]);
    for sub in ["apps/web", "services/api"] {
        let (cc, cout) = run_in(&dir.join(sub), ["check"]);
        assert_eq!(cc, 0, "{sub} nested check must pass: {cout}");
    }
}

#[test]
fn adopted_gate_passes_after_recursive_init() {
    let dir = fixture_repo("monorepo-full");
    let _ = run_in(&dir, ["init", "--non-interactive", "--recursive"]);
    let manifest = std::fs::read_to_string(dir.join(".engineering/subprojects.yml"))
        .expect("manifest")
        .replace(
            "  apps/web:\n    purpose: \"\"\n    status: pending",
            "  apps/web:\n    purpose: Web app\n    status: adopted",
        );
    std::fs::write(dir.join(".engineering/subprojects.yml"), &manifest).expect("write");
    let (cs, sout) = run_in(&dir, ["sync"]);
    assert_eq!(cs, 0, "{sout}");
    let (cc, cout) = run_in(&dir, ["check"]);
    assert_eq!(cc, 0, "grounded adoption must pass root check: {cout}");
}

#[test]
fn init_from_a_subdir_resolves_the_parent_root() {
    let dir = fixture_repo("monorepo-full");
    let web = dir.join("apps/web");
    let _ = run_in(&web, ["init", "--non-interactive"]);
    assert!(
        dir.join(".engineering/aicontext.toml").is_file(),
        "parent root must be initialized"
    );
    assert!(
        !web.join(".engineering").exists(),
        "no nested context created without --recursive"
    );
}
