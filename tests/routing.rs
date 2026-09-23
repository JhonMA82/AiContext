//! Marked `AGENTS.md` routing blocks and the subproject table contract.
//!
//! These tests lock the deterministic placement of the `aicontext:routing`
//! and `aicontext:context` blocks, the single subproject table projected
//! into both `AGENTS.md` and `.engineering/PROJECT_STATE.md`, and the
//! user-owned-file invariants: bytes outside the markers are never touched,
//! `sync` never creates the file, and the legacy unmarked pointer is
//! reported instead of rewritten or duplicated. All fixtures are committed
//! content; nothing here touches the network.

use std::path::{Path, PathBuf};
use std::process::Command;

const ROUTING_START: &str = "<!-- aicontext:routing:start -->";
const ROUTING_END: &str = "<!-- aicontext:routing:end -->";
const CONTEXT_START: &str = "<!-- aicontext:context:start -->";
const CONTEXT_END: &str = "<!-- aicontext:context:end -->";
const TABLE_HEADER: &str = "| Path | Stack | Manifest | Entry context | Commands | Purpose |";

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

fn run_with_home<const N: usize>(dir: &Path, home: &Path, args: [&str; N]) -> (i32, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
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
        "aicontext-routing-{tag}-{}-{n}-{:?}",
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

/// Builds a synthetic temp repo from `(relative path, contents)` pairs.
fn repo_with(files: &[(&str, &str)]) -> PathBuf {
    let base = temp_base("synth");
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

/// Synthetic monorepo: one root manifest declaring `apps/*` plus two
/// manifest-backed subprojects, and any extra fixture files.
fn monorepo(extra: &[(&str, &str)]) -> PathBuf {
    let mut files: Vec<(&str, &str)> = vec![
        (
            "package.json",
            r#"{"name":"root","private":true,"workspaces":["apps/*"]}"#,
        ),
        ("apps/api/package.json", r#"{"name":"api"}"#),
        ("apps/web/package.json", r#"{"name":"web"}"#),
    ];
    files.extend_from_slice(extra);
    repo_with(&files)
}

fn fresh_home(tag: &str) -> PathBuf {
    temp_base(&format!("home-{tag}"))
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(path, content).expect("write");
}

fn init(dir: &Path) -> (i32, String) {
    run(dir, ["init", "--non-interactive"])
}

#[test]
fn init_and_sync_are_byte_identical_across_runs() {
    let dir = fixture_repo("monorepo-full");
    // First init creates AGENTS.md, which becomes a scanned doc, so the
    // trailing check may report STALE_PROJECT_STATE until the next sync
    // converges (same self-referential growth as any new important path).
    let _ = init(&dir);
    let agents = read(&dir, "AGENTS.md");
    assert!(agents.contains(ROUTING_START) && agents.contains(ROUTING_END));
    assert!(agents.contains(CONTEXT_START) && agents.contains(CONTEXT_END));
    assert!(agents.contains("This repository has 4 subprojects."));

    let (cs, sout) = run(&dir, ["sync"]);
    assert_eq!(cs, 0, "sync must converge after init: {sout}");
    let agents_before = read(&dir, "AGENTS.md");
    let state_before = read(&dir, ".engineering/PROJECT_STATE.md");
    assert!(state_before.contains("| apps/desktop | npm |"));

    let (ci, _) = init(&dir);
    assert_eq!(ci, 0, "second init must pass on a converged tree");
    let (cs2, _) = run(&dir, ["sync"]);
    assert_eq!(cs2, 0);
    assert_eq!(
        read(&dir, "AGENTS.md"),
        agents_before,
        "AGENTS.md must be byte-identical across runs"
    );
    assert_eq!(
        read(&dir, ".engineering/PROJECT_STATE.md"),
        state_before,
        "PROJECT_STATE.md must be byte-identical across runs"
    );
}

#[test]
fn routing_block_is_inserted_after_the_first_h1() {
    let dir = monorepo(&[("AGENTS.md", "# Fixture repo\n\nUser prose stays here.\n")]);
    let (_, out) = init(&dir);
    let text = read(&dir, "AGENTS.md");
    assert!(
        text.starts_with(&format!("# Fixture repo\n\n{ROUTING_START}")),
        "routing block must sit immediately after the first heading:\n{text}"
    );
    let routing = text.find(ROUTING_START).expect("routing start");
    let context = text.find(CONTEXT_START).expect("context start");
    let prose = text.find("User prose stays here.").expect("user prose");
    assert!(
        routing < context && context < prose,
        "expected routing, then context, then the user bytes:\n{text}"
    );
    assert!(out.contains("inserted after the first heading"), "{out}");
}

#[test]
fn no_heading_places_routing_at_top_and_pointer_at_end() {
    let dir = monorepo(&[("AGENTS.md", "User prose without heading.\n")]);
    let (_, out) = init(&dir);
    let text = read(&dir, "AGENTS.md");
    assert!(
        text.starts_with(ROUTING_START),
        "routing must open the file"
    );
    assert!(
        text.trim_end().ends_with(CONTEXT_END),
        "pointer must close the file"
    );
    assert!(text.contains("User prose without heading."));
    assert!(out.contains("inserted at the top"), "{out}");
    assert!(out.contains("appended at the end"), "{out}");
}

#[test]
fn bytes_outside_the_marked_block_are_untouched() {
    let dir = monorepo(&[]);
    let _ = init(&dir);
    let (cs, _) = run(&dir, ["sync"]);
    assert_eq!(cs, 0);
    // Stale block with sentinels on both sides: sync must replace only the
    // marked region (and must not insert a context block on its own).
    let stale = format!(
        "# Keep\n\nBEFORE-SENTINEL\n\n{ROUTING_START}\nstale table here\n{ROUTING_END}\n\nAFTER-SENTINEL\n"
    );
    write(&dir, "AGENTS.md", &stale);
    let (cs2, out) = run(&dir, ["sync"]);
    assert_eq!(cs2, 0, "{out}");
    let after = read(&dir, "AGENTS.md");
    let prefix_len = stale.find(ROUTING_START).expect("routing start");
    let suffix_start = stale.find(ROUTING_END).expect("routing end") + ROUTING_END.len();
    assert_eq!(&after[..prefix_len], &stale[..prefix_len]);
    assert_eq!(
        &after[after.len() - (stale.len() - suffix_start)..],
        &stale[suffix_start..]
    );
    assert!(after.contains(TABLE_HEADER), "block must be refreshed");
    assert!(!after.contains("stale table here"));
    assert!(
        !after.contains(CONTEXT_START),
        "sync must report a missing pointer, never insert it"
    );
    assert!(out.contains("routing block refreshed in place"), "{out}");
}

#[test]
fn only_two_or_more_subprojects_create_the_file() {
    // Zero subprojects: a single-project repo gets no router at all.
    let dir = fixture_repo("node-single");
    let (c, out) = init(&dir);
    assert_eq!(c, 0, "{out}");
    assert!(!dir.join("AGENTS.md").exists());
    assert!(out.contains("fewer than 2 subprojects"), "{out}");

    // Exactly one declared subproject: still no router.
    let one = repo_with(&[
        (
            "package.json",
            r#"{"name":"root","private":true,"workspaces":["apps/web"]}"#,
        ),
        ("apps/web/package.json", r#"{"name":"web"}"#),
    ]);
    let _ = init(&one);
    assert!(!one.join("AGENTS.md").exists());

    // One subproject with an existing AGENTS.md: the file is left intact.
    let one_with_file = repo_with(&[
        (
            "package.json",
            r#"{"name":"root","private":true,"workspaces":["apps/web"]}"#,
        ),
        ("apps/web/package.json", r#"{"name":"web"}"#),
        ("AGENTS.md", "# Mine\n\nNo router here.\n"),
    ]);
    let _ = init(&one_with_file);
    assert_eq!(
        read(&one_with_file, "AGENTS.md"),
        "# Mine\n\nNo router here.\n"
    );
}

#[test]
fn table_cells_escape_pipes_collapse_whitespace_and_clip() {
    let long = "a".repeat(130);
    let yml = format!(
        "schema: aicontext/subprojects/v1\nsubprojects:\n  apps/web:\n    purpose: \"left | right\\nsecond line\"\n    status: pending\n  apps/api:\n    purpose: \"{long}\"\n    status: pending\n"
    );
    let dir = monorepo(&[
        (".engineering/subprojects.yml", yml.as_str()),
        ("apps/we|ird/package.json", r#"{"name":"weird"}"#),
    ]);
    let _ = init(&dir);
    let text = read(&dir, "AGENTS.md");
    assert!(
        text.contains(r"| apps/we\|ird | npm | apps/we\|ird/package.json"),
        "pipes in paths must be escaped:\n{text}"
    );
    assert!(
        text.contains(r"left \| right second line |"),
        "newlines collapse and pipes escape inside a cell:\n{text}"
    );
    assert!(
        text.contains(&"a".repeat(120)),
        "long cell is kept to 120 chars"
    );
    assert!(
        !text.contains(&"a".repeat(121)),
        "long cell must be clipped at 120 chars"
    );
}

#[test]
fn generated_markdown_shape_is_lint_stable() {
    let dir = fixture_repo("monorepo-full");
    let _ = init(&dir);
    let agents = read(&dir, "AGENTS.md");
    assert!(
        agents.contains(&format!("broadly.\n\n{TABLE_HEADER}")),
        "blank line before the table (MD058):\n{agents}"
    );
    assert!(agents.contains("| --- | --- | --- | --- | --- | --- |"));
    assert!(
        !agents.contains("|---|---|"),
        "MD060: compact separators are not allowed"
    );
    assert!(
        agents.contains("|\n\nRules:\n\n- Identify the subproject"),
        "blank line after the table and before the Rules list (MD032):\n{agents}"
    );

    let state = read(&dir, ".engineering/PROJECT_STATE.md");
    assert!(
        state.contains(&format!("Subprojects:\n\n{TABLE_HEADER}")),
        "the generated block carries the same table:\n{state}"
    );
    assert!(state.contains("| --- | --- | --- | --- | --- | --- |"));
    assert!(
        state.contains("|\n\n<!-- aicontext:generated:end -->"),
        "blank line after the table inside PROJECT_STATE.md"
    );
}

#[test]
fn purpose_change_is_freshness_drift_until_sync() {
    let yml = |purpose: &str| {
        format!(
            "schema: aicontext/subprojects/v1\nsubprojects:\n  apps/api:\n    purpose: First purpose\n    status: pending\n  apps/web:\n    purpose: {purpose}\n    status: pending\n"
        )
    };
    let dir = monorepo(&[(".engineering/subprojects.yml", &yml("Second purpose"))]);
    let _ = init(&dir);
    let (cs, _) = run(&dir, ["sync"]);
    assert_eq!(cs, 0);
    assert!(read(&dir, ".engineering/PROJECT_STATE.md").contains("Second purpose"));
    let (c0, _) = run(&dir, ["sync", "--check"]);
    assert_eq!(c0, 0);

    // `purpose` is a projected fact: editing it is genuine drift.
    write(
        &dir,
        ".engineering/subprojects.yml",
        &yml("Renamed purpose"),
    );
    let (c1, out1) = run(&dir, ["sync", "--check"]);
    assert_eq!(c1, 1, "changed purpose must report stale: {out1}");
    let (c2, out2) = run(&dir, ["sync"]);
    assert_eq!(c2, 0, "{out2}");
    let state = read(&dir, ".engineering/PROJECT_STATE.md");
    assert!(state.contains("Renamed purpose"));
    assert!(!state.contains("Second purpose"));
    assert!(
        read(&dir, "AGENTS.md").contains("Renamed purpose"),
        "the routing block is refreshed in the same sync"
    );
    let (c3, out3) = run(&dir, ["sync", "--check"]);
    assert_eq!(c3, 0, "synced tree must be green: {out3}");
}

#[test]
fn legacy_pointer_is_reported_neither_duplicated_nor_rewritten() {
    let dir = monorepo(&[("AGENTS.md", "# Test repo\n")]);
    let home = fresh_home("legacy");
    // `agent install` appends the unmarked pointer this test must protect.
    let (ca, outa) = run_with_home(&dir, &home, ["agent", "install", "pi"]);
    assert_eq!(ca, 0, "{outa}");
    let legacy = read(&dir, "AGENTS.md");
    assert!(legacy.contains("## Repository context"));

    let (_, out) = init(&dir);
    let after = read(&dir, "AGENTS.md");
    assert!(out.contains("legacy pointer left untouched"), "{out}");
    assert_eq!(
        after.matches("## Repository context").count(),
        1,
        "the legacy pointer must not be duplicated"
    );
    assert!(
        !after.contains(CONTEXT_START),
        "the legacy pointer must not be rewritten into markers"
    );
    assert_eq!(after.matches("`aicontext check`").count(), 1);
    assert!(after.contains(ROUTING_START), "the router is still added");

    let (_, out2) = init(&dir);
    assert_eq!(read(&dir, "AGENTS.md"), after);
    assert!(out2.contains("legacy pointer left untouched"), "{out2}");
}

#[test]
fn routing_row_addition_converges_in_one_sync() {
    let dir = monorepo(&[]);
    let _ = init(&dir);
    let (cs, sout) = run(&dir, ["sync"]);
    assert_eq!(cs, 0, "sync must converge after init: {sout}");
    // Topology change: a third subproject plus its adoption entry.
    write(&dir, "apps/third/package.json", r#"{"name":"third"}"#);
    git(&dir, ["add", "-A"]);
    git(&dir, ["commit", "-qm", "third subproject"]);
    let manifest = read(&dir, ".engineering/subprojects.yml");
    assert!(manifest.contains("apps/api"));
    write(
        &dir,
        ".engineering/subprojects.yml",
        &format!("{manifest}  apps/third:\n    purpose: \"\"\n    status: pending\n"),
    );
    // One sync must converge: the routing refresh (new row) is re-observed
    // before the generated block is written, otherwise Markdown LOC drift
    // leaves the tree stale until a second pass.
    let (cs2, out2) = run(&dir, ["sync"]);
    assert_eq!(cs2, 0, "{out2}");
    let (cc, cout) = run(&dir, ["sync", "--check"]);
    assert_eq!(cc, 0, "one sync must converge after a row addition: {cout}");
    let state = read(&dir, ".engineering/PROJECT_STATE.md");
    assert!(state.contains("apps/third"), "new row projected:\n{state}");
    let agents = read(&dir, "AGENTS.md");
    assert!(
        agents.contains("apps/third"),
        "routing refreshed:\n{agents}"
    );
}
