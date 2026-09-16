use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
}

fn run<const N: usize>(dir: &Path, args: [&str; N]) -> (i32, String) {
    run_with_path(dir, args, std::env::var("PATH").unwrap_or_default())
}

fn run_with_path<const N: usize>(dir: &Path, args: [&str; N], path: String) -> (i32, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .env("PATH", path)
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
        .env("GIT_COMMITTER_NAME", "t@t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .expect("git");
    assert!(st.success(), "git {args:?} failed");
}

fn unique_base(prefix: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-{prefix}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    std::fs::create_dir_all(&base).expect("mkdir");
    base
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
    let base = unique_base(&format!("adapters-{name}"));
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    copy_dir(&manifest.join("fixtures").join(name), &base);
    git(&base, ["init", "-q"]);
    git(&base, ["add", "-A"]);
    git(&base, ["commit", "-qm", "fixture"]);
    base
}

/// A fake tool on PATH: answers `--version` (so detection succeeds) and
/// prints canned stdout with the given exit code otherwise.
fn write_shim(bin_dir: &Path, name: &str, stdout: &str, exit: i32) {
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"{name} test-0.0.0\"; exit 0; fi\ncat \"$0.payload\"; exit {exit}\n"
    );
    std::fs::write(bin_dir.join(name), script).expect("shim");
    std::fs::write(bin_dir.join(format!("{name}.payload")), stdout).expect("payload");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = bin_dir.join(name);
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
}

fn overlay_path(bin_dir: &Path) -> String {
    format!(
        "{}:{}",
        bin_dir.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    )
}

fn adapter_findings(check_json: &serde_json::Value) -> Vec<(String, bool, String)> {
    let Some(findings) = check_json.get("findings").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    findings
        .iter()
        .filter_map(|f| {
            let name = f.get("name")?.as_str()?;
            if name.starts_with("adapter:") {
                Some((
                    name.to_string(),
                    f.get("passed")?.as_bool()?,
                    f.get("detail")?.as_str()?.to_string(),
                ))
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn knip_findings_are_evidence_and_never_block() {
    let dir = fixture_repo("node-single");
    let bindir = unique_base("adapters-bin-knip");
    // Knip exits 1 when it reports issues; the adapter must still parse stdout.
    write_shim(
        &bindir,
        "knip",
        r#"{"files": ["src/old.ts"], "exports": {"src/a.ts": ["unusedFn"]}}"#,
        1,
    );
    let path = overlay_path(&bindir);
    let (c, _) = run_with_path(&dir, ["init", "--non-interactive"], path.clone());
    assert_eq!(c, 0, "init must pass: adapter findings never block");
    let (cc, out) = run_with_path(&dir, ["check", "--json"], path);
    assert_eq!(cc, 0);
    let v: serde_json::Value = serde_json::from_str(&out).expect("check --json");
    assert_eq!(v["schema"], "aicontext/check/v1");
    let findings = adapter_findings(&v);
    let knip = findings
        .iter()
        .find(|(n, _, _)| n == "adapter:knip")
        .expect("knip applies to package.json repos");
    assert!(knip.1, "adapter findings always pass");
    assert!(
        knip.2.contains("2 issue(s)"),
        "must count files + exports: {}",
        knip.2
    );
}

#[test]
fn missing_binaries_degrade_to_notes() {
    let dir = fixture_repo("node-single");
    // Restricted PATH: only git resolves, so every P4 probe reports missing.
    let bindir = unique_base("adapters-bin-empty");
    let git_path = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("locate git");
    let git_bin = String::from_utf8_lossy(&git_path.stdout).trim().to_string();
    assert!(!git_bin.is_empty(), "git must exist for fixtures");
    std::os::unix::fs::symlink(&git_bin, bindir.join("git")).expect("link git");
    let path = bindir.to_string_lossy().to_string();
    let (c, _) = run_with_path(&dir, ["init", "--non-interactive"], path.clone());
    assert_eq!(c, 0, "init must pass without P4 binaries");
    let (cc, out) = run_with_path(&dir, ["check", "--json"], path);
    assert_eq!(cc, 0);
    let v: serde_json::Value = serde_json::from_str(&out).expect("check --json");
    let findings = adapter_findings(&v);
    let names: Vec<&str> = findings.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"adapter:knip"), "knip applies: {names:?}");
    assert!(
        names.contains(&"adapter:dependency-cruiser"),
        "depcruiser applies: {names:?}"
    );
    for (_, passed, detail) in &findings {
        assert!(passed, "missing tools never fail: {detail}");
        assert!(
            detail.contains("absent") || detail.contains("no boundaries config"),
            "must explain the gap: {detail}"
        );
    }
}

#[test]
fn adapters_are_skipped_where_they_do_not_apply() {
    let dir = fixture_repo("rust-single");
    let (c, _) = run(&dir, ["init", "--non-interactive"]);
    let _ = c; // rust fixture may fail the version-source check; files still land.
    assert!(dir.join(".engineering/aicontext.toml").exists());
    let (_, cout) = run(&dir, ["check", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&cout).expect("check --json");
    let findings = adapter_findings(&v);
    assert!(
        findings.is_empty(),
        "Rust fixture triggers no adapter: {findings:?}"
    );
}

#[test]
fn plan_lists_dependency_cruiser_for_js() {
    let dir = fixture_repo("node-single");
    let (c, out) = run(&dir, ["tools", "plan", "--json"]);
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).expect("plan --json");
    let mut tools: Vec<(String, String)> = Vec::new();
    if let Some(sections) = v.get("sections").and_then(|s| s.as_array()) {
        for s in sections {
            if let Some(entries) = s.get("entries").and_then(|e| e.as_array()) {
                for e in entries {
                    if let (Some(tool), Some(level)) = (
                        e.get("tool").and_then(|t| t.as_str()),
                        e.get("level").and_then(|l| l.as_str()),
                    ) {
                        tools.push((tool.to_string(), level.to_string()));
                    }
                }
            }
        }
    }
    assert!(
        tools.contains(&("dependency-cruiser".to_string(), "adapter".to_string())),
        "plan must recommend dependency-cruiser: {tools:?}"
    );
}

#[test]
fn full_p4_counts_every_adapter() {
    let dir = unique_base("adapters-full");
    std::fs::write(
        dir.join("package.json"),
        r#"{"name": "x", "version": "0.1.0"}"#,
    )
    .unwrap();
    std::fs::write(dir.join(".dependency-cruiser.json"), r#"{"forbidden": []}"#).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("docs/guide.md"), "# g\n[ok](guide.md)\n").unwrap();
    std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
    std::fs::write(dir.join(".github/workflows/ci.yml"), "on: push\njobs: {}\n").unwrap();
    git(&dir, ["init", "-q"]);
    git(&dir, ["add", "-A"]);
    git(&dir, ["commit", "-qm", "fixture"]);

    let bindir = unique_base("adapters-bin-full");
    write_shim(
        &bindir,
        "knip",
        r#"{"dependencies": {"lodash": {}}, "unlisted": {"src/x.ts": ["y"]}}"#,
        1,
    );
    write_shim(
        &bindir,
        "depcruise",
        r#"{"summary": {"violations": [{"rule": {"name": "no-circular"}}], "error": 1, "warn": 0}}"#,
        0,
    );
    write_shim(
        &bindir,
        "lychee",
        r#"{"total": 3, "successful": 2, "errors": 1, "error_map": {"docs/guide.md": [{"url": "missing.md"}]}}"#,
        2,
    );
    write_shim(
        &bindir,
        "zizmor",
        r#"[{"ident": "template-injection", "determinations": {"severity": "High", "confidence": "High"}}]"#,
        1,
    );
    let path = overlay_path(&bindir);
    let (c, _) = run_with_path(&dir, ["init", "--non-interactive"], path.clone());
    assert_eq!(c, 0, "full P4 evidence must not block init");
    let (cc, out) = run_with_path(&dir, ["check", "--json"], path);
    assert_eq!(cc, 0);
    let v: serde_json::Value = serde_json::from_str(&out).expect("check --json");
    let findings = adapter_findings(&v);
    let get = |name: &str| {
        findings
            .iter()
            .find(|(n, _, _)| n == name)
            .unwrap_or_else(|| panic!("missing {name}: {findings:?}"))
            .clone()
    };
    let (_, kp, kd) = get("adapter:knip");
    let (_, dp, dd) = get("adapter:dependency-cruiser");
    let (_, lp, ld) = get("adapter:lychee");
    let (_, zp, zd) = get("adapter:zizmor");
    assert!(kp && dp && lp && zp, "all adapter findings pass");
    assert!(kd.contains("2 issue(s)"), "knip count: {kd}");
    assert!(dd.contains("1 violation(s)"), "depcruise count: {dd}");
    assert!(dd.contains("no-circular"), "rule name as evidence: {dd}");
    assert!(ld.contains("1 broken link(s)"), "lychee count: {ld}");
    assert!(zd.contains("template-injection"), "zizmor ident: {zd}");
}
