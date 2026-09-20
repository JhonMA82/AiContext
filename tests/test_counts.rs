//! Deterministic test-function census: the generated block carries a
//! `Test functions: N (src: A, tests: B)` line computed from tracked
//! `*.rs` files, so adding a test function is genuine drift (`sync --check`
//! fails) and the next `sync` converges back to green.

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
        "aicontext-counts-{name}-{}-{n}-{:?}",
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

fn state_text(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(".engineering/PROJECT_STATE.md")).expect("read state")
}

/// Parse `Test functions: N (src: A, tests: B)`; asserts shape and that the
/// total equals the sum of the parts. Returns (total, src, tests).
fn parse_test_line(state: &str) -> (u64, u64, u64) {
    let line = state
        .lines()
        .find(|l| l.trim_start().starts_with("Test functions:"))
        .expect("generated block must contain a Test functions line");
    let after = line
        .trim_start()
        .strip_prefix("Test functions:")
        .expect("prefix")
        .trim();
    let open = after.find('(').expect("open paren");
    let close = after.find(')').expect("close paren");
    assert!(
        open < close,
        "parens must be ordered in Test functions line"
    );
    let total: u64 = after[..open].trim().parse().expect("total is a number");
    let mut unit: Option<u64> = None;
    let mut integration: Option<u64> = None;
    for part in after[open + 1..close].split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("src:") {
            unit = Some(rest.trim().parse().expect("src is a number"));
        } else if let Some(rest) = part.strip_prefix("tests:") {
            integration = Some(rest.trim().parse().expect("tests is a number"));
        } else {
            panic!("unexpected segment in Test functions line: {part}");
        }
    }
    let (u, i) = (
        unit.expect("src segment"),
        integration.expect("tests segment"),
    );
    assert_eq!(total, u + i, "total must equal src + tests");
    (total, u, i)
}

#[test]
fn generated_block_contains_well_formed_test_functions_line() {
    let dir = fixture_repo("rust-single");
    // rust-single has no version source; init's trailing check may fail, but
    // the state files must still be generated.
    let _ = run(&dir, ["init", "--non-interactive"]);
    let (cs, out) = run(&dir, ["sync"]);
    assert_eq!(cs, 0, "sync must succeed: {out}");
    let before = state_text(&dir);
    let (total, unit, integration) = parse_test_line(&before);
    assert_eq!(total, unit + integration);
    // Determinism: a second sync is a zero diff.
    let (cs2, _) = run(&dir, ["sync"]);
    assert_eq!(cs2, 0);
    let after = state_text(&dir);
    assert_eq!(before, after, "sync must be deterministic");
    // Lint stability (generated block only): every markdown list item
    // the generator emits follows a blank line, so a formatter never
    // rewrites sync output (a rewrite inside the generated block would
    // invalidate freshness by construction and make `check` STALE
    // forever). The curated block is human-owned and out of scope here.
    let gen_start = before
        .find("<!-- aicontext:generated:start -->")
        .expect("generated start marker");
    let gen_end = before
        .find("<!-- aicontext:generated:end -->")
        .expect("generated end marker");
    let generated = before
        .get(gen_start..gen_end)
        .expect("generated block slice");
    let lines: Vec<&str> = generated.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("- ") {
            assert!(
                i == 0 || lines[i - 1].trim().is_empty(),
                "list item must follow a blank line, found: {:?}",
                lines.get(i.saturating_sub(1))
            );
        }
    }
}

#[test]
fn adding_one_test_function_breaks_check_and_sync_converges() {
    let dir = fixture_repo("rust-single");
    let _ = run(&dir, ["init", "--non-interactive"]);
    let (cs, out) = run(&dir, ["sync"]);
    assert_eq!(cs, 0, "sync must succeed: {out}");
    let (_, base_unit, base_integration) = parse_test_line(&state_text(&dir));

    std::fs::create_dir_all(dir.join("tests")).expect("mkdir tests");
    std::fs::write(
        dir.join("tests/extra_count.rs"),
        "#[test]\nfn counted_probe() {\n    assert_eq!(1 + 1, 2);\n}\n",
    )
    .expect("write probe");
    git(&dir, ["add", "-A"]);
    git(&dir, ["commit", "-qm", "one more test fn"]);

    let (cc, cout) = run(&dir, ["sync", "--check"]);
    assert_eq!(cc, 1, "new test fn is drift: {cout}");
    let (cs2, sout) = run(&dir, ["sync"]);
    assert_eq!(cs2, 0, "sync must converge: {sout}");
    let (cc2, cout2) = run(&dir, ["sync", "--check"]);
    assert_eq!(cc2, 0, "synced tree must be green: {cout2}");

    let (_, unit, integration) = parse_test_line(&state_text(&dir));
    assert_eq!(unit, base_unit, "src count is unchanged");
    assert_eq!(
        integration,
        base_integration + 1,
        "tests count grows by exactly one"
    );
}
