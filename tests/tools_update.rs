use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
}

fn run_with_env<const N: usize>(
    dir: &Path,
    home: &Path,
    path_extra: Option<&Path>,
    args: [&str; N],
) -> (i32, String) {
    let mut cmd = Command::new(bin());
    cmd.args(args).current_dir(dir).env("HOME", home);
    if let Some(extra) = path_extra {
        let base = std::env::var("PATH").unwrap_or_default();
        cmd.env("PATH", format!("{}:{base}", extra.display()));
    }
    let out = cmd.output().expect("run aicontext");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(255), text)
}

fn fresh_case(tag: &str) -> (PathBuf, PathBuf) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-tmanage-{tag}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    let home = base.join("home");
    let work = base.join("work");
    std::fs::create_dir_all(&home).expect("mkdir");
    std::fs::create_dir_all(&work).expect("mkdir");
    (home, work)
}

fn prefix(home: &Path) -> PathBuf {
    home.join(".local/share/aicontext")
}

fn write_registry(home: &Path, body: &str) {
    let dir = prefix(home);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("managed-tools.json"), body).expect("write");
}

fn managed_entry(name: &str, method: &str, path: &str) -> String {
    format!(
        r#"{{"name":"{name}","version":"0.0-test","method":"{method}","path":"{path}","installed_at":1,"managed_by":"managed"}}"#
    )
}

#[test]
fn update_without_managed_tools_is_noop() {
    let (home, work) = fresh_case("empty");
    let (c, out) = run_with_env(&work, &home, None, ["tools", "update", "--json"]);
    assert_eq!(c, 0, "empty update must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["schema"], "aicontext/tools-update/v1");
    assert!(v["updated"].as_array().unwrap().is_empty());
    assert!(
        v["note"].as_str().unwrap().contains("no managed tools"),
        "note must explain: {v:?}"
    );
}

#[test]
fn update_skips_entries_without_safe_method() {
    let (home, work) = fresh_case("skip");
    // Hand-placed entry with no cargo method: must be reported, never run.
    write_registry(
        &home,
        &format!(
            r#"{{"tools":[{}]}}"#,
            managed_entry("tgrep", "manual", "/nonexistent/tgrep")
        ),
    );
    let (c, out) = run_with_env(&work, &home, None, ["tools", "update", "--yes", "--json"]);
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["updated"].as_array().unwrap().is_empty());
    assert_eq!(v["skipped"][0]["tool"], "tgrep");
}

#[test]
fn update_without_yes_declines_gracefully() {
    let (home, work) = fresh_case("decline");
    write_registry(
        &home,
        &format!(
            r#"{{"tools":[{}]}}"#,
            managed_entry(
                "lychee",
                "cargo install lychee --locked",
                "/nonexistent/lychee"
            )
        ),
    );
    let before = std::fs::read(prefix(&home).join("managed-tools.json")).unwrap();
    let (c, out) = run_with_env(&work, &home, None, ["tools", "update"]);
    assert_eq!(c, 0, "decline is not a failure: {out}");
    assert!(out.to_lowercase().contains("declin"));
    let after = std::fs::read(prefix(&home).join("managed-tools.json")).unwrap();
    assert_eq!(before, after, "declines must not touch the registry");
}

#[test]
fn uninstall_unknown_tool_fails_closed() {
    let (home, work) = fresh_case("unknown");
    let (c, _) = run_with_env(&work, &home, None, ["tools", "uninstall", "nope", "--json"]);
    assert_eq!(c, 2);
}

#[test]
fn uninstall_usage_error_without_target() {
    let (home, work) = fresh_case("usage");
    let (c, _) = run_with_env(&work, &home, None, ["tools", "uninstall", "--json"]);
    assert_eq!(c, 2);
}

#[test]
fn uninstall_absent_tool_is_noop() {
    let (home, work) = fresh_case("noop");
    let (c, out) = run_with_env(
        &work,
        &home,
        None,
        ["tools", "uninstall", "lychee", "--yes", "--json"],
    );
    assert_eq!(c, 0, "absent noop must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["schema"], "aicontext/tools-uninstall/v1");
    assert!(v["removed"].as_array().unwrap().is_empty());
    assert_eq!(v["not_installed"], serde_json::json!(["lychee"]));
}

#[test]
fn uninstall_removes_only_owned_files() {
    let (home, work) = fresh_case("owned");
    let bin_path = prefix(&home).join("bin/fake-tool");
    std::fs::create_dir_all(bin_path.parent().unwrap()).expect("mkdir");
    std::fs::write(&bin_path, "fake").expect("write");
    write_registry(
        &home,
        &format!(
            r#"{{"tools":[{}]}}"#,
            managed_entry(
                "lychee",
                "cargo install lychee --locked",
                &bin_path.to_string_lossy()
            )
        ),
    );
    let (c, out) = run_with_env(
        &work,
        &home,
        None,
        ["tools", "uninstall", "lychee", "--yes", "--json"],
    );
    assert_eq!(c, 0, "owned removal must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["removed"][0]["tool"], "lychee");
    assert!(bin_path.exists() == false, "owned file must be deleted");
    let reg: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(prefix(&home).join("managed-tools.json")).unwrap(),
    )
    .unwrap();
    assert!(reg["tools"].as_array().unwrap().is_empty());
}

#[test]
fn uninstall_refuses_external_installs() {
    let (home, work) = fresh_case("external");
    let fake_bin = work.join("fakebin");
    std::fs::create_dir_all(&fake_bin).expect("mkdir");
    let stub = fake_bin.join("tgrep");
    std::fs::write(&stub, "#!/bin/sh\necho \"stub 0.0-test\"\n").expect("write");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let (c, out) = run_with_env(
        &work,
        &home,
        Some(&fake_bin),
        ["tools", "uninstall", "tgrep", "--yes", "--json"],
    );
    assert_eq!(c, 1, "refusal must exit 1: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["refused"][0]["tool"], "tgrep");
    assert!(stub.exists(), "external files must survive");
}

#[test]
fn uninstall_managed_removes_everything_owned() {
    let (home, work) = fresh_case("all");
    let mut entries = Vec::new();
    for name in ["lychee", "zizmor"] {
        let p = prefix(&home).join(format!("bin/{name}"));
        std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
        std::fs::write(&p, "fake").expect("write");
        entries.push(managed_entry(
            name,
            &format!("cargo install {name} --locked"),
            &p.to_string_lossy(),
        ));
    }
    write_registry(&home, &format!(r#"{{"tools":[{}]}}"#, entries.join(",")));
    let (c, out) = run_with_env(
        &work,
        &home,
        None,
        ["tools", "uninstall", "--managed", "--yes", "--json"],
    );
    assert_eq!(c, 0, "--managed must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["removed"].as_array().unwrap().len(), 2);
    assert!(prefix(&home).join("bin/lychee").exists() == false);
    assert!(prefix(&home).join("bin/zizmor").exists() == false);
}
