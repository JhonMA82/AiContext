//! P0 consistency gate: `check` must verify declarations against reality.
//! - `commands.documented` matches detected scripts both ways.
//! - `version.projections` files carry the resolved version.
//! - Declared ast-grep rules are executed; a match fails.

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
        "aicontext-cons-{name}-{}-{n}-{:?}",
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

fn finding(out: &str, name: &str) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(out).expect("check --json must be JSON");
    v.get("findings")
        .and_then(|f| f.as_array())
        .and_then(|fs| {
            fs.iter()
                .find(|f| f.get("name").and_then(|n| n.as_str()) == Some(name))
                .cloned()
        })
        .unwrap_or_else(|| panic!("finding {name} must exist: {out}"))
}

fn set_projections(dir: &Path, yaml_list: &str) {
    let path = dir.join(".engineering/consistency.yml");
    let text = std::fs::read_to_string(&path).unwrap();
    let patched = text.replace("projections: []", yaml_list);
    assert!(patched != text, "projections stub must exist");
    std::fs::write(&path, patched).unwrap();
}

#[test]
fn init_seeds_documented_commands_and_check_passes() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0, "fresh init must pass check");
    let yml = std::fs::read_to_string(dir.join(".engineering/consistency.yml")).unwrap();
    assert!(
        yml.contains("generate:feature"),
        "init must seed scripts:\n{yml}"
    );
    assert!(yml.contains("- test"), "init must seed scripts:\n{yml}");
    let (_, out) = run(&dir, ["check", "--json"]);
    let f = finding(&out, "commands");
    assert_eq!(f.get("passed").and_then(|p| p.as_bool()), Some(true), "{f}");
}

#[test]
fn removed_command_fails_with_stale_doc_mention() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    // Drop the script while the README still references it: exactly the drift
    // the gate must catch.
    let pkg = dir.join("package.json");
    let text = std::fs::read_to_string(&pkg).unwrap();
    let patched = text.replace(",\n    \"generate:feature\": \"node scripts/gen.js\"", "");
    assert!(patched != text);
    std::fs::write(&pkg, patched).unwrap();
    std::fs::write(
        dir.join("README.md"),
        "# node-single fixture\n\nRun `generate:feature` to scaffold.\n",
    )
    .unwrap();

    let (code, out) = run(&dir, ["check", "--json"]);
    assert_eq!(code, 1, "stale command must fail check: {out}");
    let f = finding(&out, "commands");
    assert_eq!(
        f.get("passed").and_then(|p| p.as_bool()),
        Some(false),
        "{f}"
    );
    let detail = f.get("detail").and_then(|d| d.as_str()).unwrap_or("");
    assert!(
        detail.contains("stale"),
        "must name the stale entry: {detail}"
    );
    assert!(detail.contains("generate:feature"), "{detail}");
    assert!(
        detail.contains("README.md"),
        "must point at the stale mention: {detail}"
    );
}

#[test]
fn undocumented_command_fails_check() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    let pkg = dir.join("package.json");
    let text = std::fs::read_to_string(&pkg).unwrap();
    let patched = text.replace(
        "\"test\": \"node --test\"",
        "\"test\": \"node --test\",\n    \"brand:new\": \"node scripts/new.js\"",
    );
    assert!(patched != text);
    std::fs::write(&pkg, patched).unwrap();

    let (code, out) = run(&dir, ["check", "--json"]);
    assert_eq!(code, 1, "undocumented command must fail check: {out}");
    let f = finding(&out, "commands");
    assert_eq!(
        f.get("passed").and_then(|p| p.as_bool()),
        Some(false),
        "{f}"
    );
    assert!(
        f.get("detail")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .contains("undocumented"),
        "{f}"
    );
}

#[test]
fn version_projection_mismatch_fails_and_fix_passes() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    // README pins nothing; declaring it as a projection must fail.
    set_projections(&dir, "projections:\n      - README.md");
    let (code, out) = run(&dir, ["check", "--json"]);
    assert_eq!(code, 1, "projection without the version must fail: {out}");
    let f = finding(&out, "consistency");
    assert_eq!(
        f.get("passed").and_then(|p| p.as_bool()),
        Some(false),
        "{f}"
    );

    // A file carrying the resolved version (0.1.0) satisfies the projection.
    std::fs::write(dir.join("VERSION.txt"), "0.1.0\n").unwrap();
    let path = dir.join(".engineering/consistency.yml");
    let text = std::fs::read_to_string(&path).unwrap();
    let patched = text.replace("- README.md", "- VERSION.txt");
    assert!(patched != text, "projection entry must exist");
    std::fs::write(&path, patched).unwrap();
    let (code2, out2) = run(&dir, ["check", "--json"]);
    let f2 = finding(&out2, "consistency");
    assert_eq!(
        f2.get("passed").and_then(|p| p.as_bool()),
        Some(true),
        "carried version must pass: {f2}"
    );
    let _ = code2;
}

#[test]
fn missing_projection_file_fails() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    set_projections(&dir, "projections:\n      - docs/NOPE.md");
    let (code, out) = run(&dir, ["check", "--json"]);
    assert_eq!(code, 1, "missing projection target must fail: {out}");
    assert_eq!(
        finding(&out, "consistency")
            .get("passed")
            .and_then(|p| p.as_bool()),
        Some(false)
    );
}

fn ast_grep_available() -> bool {
    Command::new("ast-grep")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn declared_rule_violation_fails_structural_gate() {
    if ast_grep_available() {
        structural_violation_case();
    } else {
        eprintln!("SKIP: ast-grep absent — structural execution untestable here");
    }
}

fn structural_violation_case() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    // The fixture's index.js logs to the console; forbid it structurally.
    std::fs::write(
        dir.join(".engineering/rules/ast-grep/no-console.yml"),
        "id: no-console-log\nlanguage: js\nrule:\n  pattern: console.log($$$)\nmessage: no console.log\nseverity: error\n",
    )
    .unwrap();
    let (code, out) = run(&dir, ["check", "--json"]);
    assert_eq!(code, 1, "rule match must fail check: {out}");
    let f = finding(&out, "structural rules");
    assert_eq!(
        f.get("passed").and_then(|p| p.as_bool()),
        Some(false),
        "{f}"
    );
    let detail = f.get("detail").and_then(|d| d.as_str()).unwrap_or("");
    assert!(
        detail.contains("no-console"),
        "must name the rule: {detail}"
    );
    assert!(detail.contains("index.js"), "must name the file: {detail}");
}

#[test]
fn clean_rules_pass_structural_gate() {
    if ast_grep_available() {
        structural_clean_case();
    } else {
        eprintln!("SKIP: ast-grep absent — structural execution untestable here");
    }
}

fn structural_clean_case() {
    let dir = fixture_repo("node-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    assert_eq!(c, 0);
    // A rule matching nothing present in the fixture stays green.
    std::fs::write(
        dir.join(".engineering/rules/ast-grep/no-debugger.yml"),
        "id: no-debugger\nlanguage: js\nrule:\n  pattern: debugger\nmessage: no debugger\nseverity: error\n",
    )
    .unwrap();
    let (code, out) = run(&dir, ["check", "--json"]);
    assert_eq!(code, 0, "clean rules must pass: {out}");
    assert_eq!(
        finding(&out, "structural rules")
            .get("passed")
            .and_then(|p| p.as_bool()),
        Some(true)
    );
}

/// Synthetic temp repo from `(relative path, contents)` pairs.
fn synth_repo(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-cons-synth-{tag}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
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

#[test]
fn init_detects_cargo_version_source_at_root() {
    let dir = synth_repo(
        "cargo-root",
        &[(
            "Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )],
    );
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    let _ = c;
    let (cs, sout) = run(&dir, ["sync"]);
    assert_eq!(cs, 0, "sync must converge: {sout}");
    let manifest = std::fs::read_to_string(dir.join(".engineering/aicontext.toml")).unwrap();
    assert!(
        manifest.contains("source = \"Cargo.toml\""),
        "root manifest must carry the detected source:\n{manifest}"
    );
    let (code, out) = run(&dir, ["check", "--json"]);
    assert_eq!(code, 0, "cargo-only root must pass check: {out}");
    assert_eq!(
        finding(&out, "version").get("passed").and_then(|p| p.as_bool()),
        Some(true)
    );
}
