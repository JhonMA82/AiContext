use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
}

fn run_with_path<const N: usize>(
    dir: &Path,
    home: &Path,
    path: String,
    args: [&str; N],
) -> (i32, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("PATH", path)
        .output()
        .expect("run aicontext");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(255), text)
}

fn run_with_env<const N: usize>(
    dir: &Path,
    home: &Path,
    path_extra: Option<&Path>,
    args: [&str; N],
) -> (i32, String) {
    let base = std::env::var("PATH").unwrap_or_default();
    let path = match path_extra {
        Some(extra) => format!("{}:{base}", extra.display()),
        None => base,
    };
    run_with_path(dir, home, path, args)
}

/// PATH pointing at an empty dir: every external probe fails deterministically,
/// in any environment, without touching the real toolchain.
fn isolated_path(home: &Path) -> String {
    let empty = home.join("emptybin");
    std::fs::create_dir_all(&empty).expect("mkdir");
    empty.to_string_lossy().into_owned()
}

fn fresh_case(tag: &str) -> (PathBuf, PathBuf) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-tinstall-{tag}-{}-{n}-{:?}",
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

fn managed_registry(home: &Path) -> PathBuf {
    home.join(".local/share/aicontext/managed-tools.json")
}

#[test]
fn unknown_tool_fails_closed() {
    let (home, work) = fresh_case("unknown");
    let (c, _) = run_with_env(&work, &home, None, ["tools", "install", "nope", "--json"]);
    assert_eq!(c, 2, "unknown tool must exit 2");
    assert!(
        managed_registry(&home).exists() == false,
        "failures must not create a registry"
    );
}

#[test]
fn usage_error_without_target() {
    let (home, work) = fresh_case("usage");
    let (c, _) = run_with_env(&work, &home, None, ["tools", "install", "--json"]);
    assert_eq!(c, 2, "missing target must exit 2");
}

#[test]
fn manual_method_is_skipped_with_reason() {
    let (home, work) = fresh_case("manual");
    // Isolated PATH: tgrep is guaranteed absent here, so its manual method
    // is exercised even on machines where tgrep is really installed.
    let path = isolated_path(&home);
    let out = Command::new(bin())
        .args(["tools", "install", "tgrep", "--yes", "--json"])
        .current_dir(&work)
        .env("HOME", &home)
        .env("PATH", path)
        .output()
        .expect("run aicontext");
    let c = out.status.code().unwrap_or(255);
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let out = text;
    assert_eq!(c, 0, "skip-only runs succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("must be JSON");
    assert_eq!(v["schema"], "aicontext/tools-install/v1");
    assert_eq!(v["requested"], serde_json::json!(["tgrep"]));
    assert!(v["installed"].as_array().unwrap().is_empty());
    let skipped = v.get("skipped").and_then(|x| x.as_array()).unwrap();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0]["tool"], "tgrep");
    assert!(
        skipped[0]["reason"].as_str().unwrap().contains("manual"),
        "skip reason must explain manual steps"
    );
    assert!(
        managed_registry(&home).exists() == false,
        "skips must not create a registry"
    );
}

#[test]
fn project_local_adapters_never_touch_manifests() {
    let (home, work) = fresh_case("local");
    let (c, out) = run_with_env(
        &work,
        &home,
        None,
        ["tools", "install", "knip", "--yes", "--json"],
    );
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["skipped"][0]["tool"], "knip");
    assert!(
        work.join("package.json").exists() == false,
        "install must never create manifests"
    );
}

#[test]
fn non_tty_without_yes_declines_gracefully() {
    let (home, work) = fresh_case("decline");
    // docs/ makes lychee (cargo method) appear as installable.
    std::fs::create_dir_all(work.join("docs")).expect("mkdir");
    let (c, out) = run_with_env(&work, &home, None, ["tools", "install", "--recommended"]);
    assert_eq!(c, 0, "decline is not a failure: {out}");
    assert!(
        out.to_lowercase().contains("declin"),
        "output must say it declined: {out}"
    );
    assert!(
        managed_registry(&home).exists() == false,
        "declines must not create a registry"
    );
}

#[test]
fn present_tools_are_reported_not_touched() {
    let (home, work) = fresh_case("present");
    // Fake PATH stubs make tgrep/rtk look installed.
    let fake_bin = work.join("fakebin");
    std::fs::create_dir_all(&fake_bin).expect("mkdir");
    for tool in ["tgrep", "rtk"] {
        let stub = fake_bin.join(tool);
        std::fs::write(&stub, "#!/bin/sh\necho \"stub 0.0-test\"\n").expect("write");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
    }
    // Explicit request for a present tool lands in already_present.
    let (c, out) = run_with_env(
        &work,
        &home,
        Some(&fake_bin),
        ["tools", "install", "tgrep", "--yes", "--json"],
    );
    assert_eq!(c, 0, "present-tool runs succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let present: Vec<&str> = v
        .get("already_present")
        .and_then(|x| x.as_array())
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str())
        .collect();
    assert!(present.contains(&"tgrep"));
    assert!(v
        .get("installed")
        .and_then(|x| x.as_array())
        .unwrap()
        .is_empty());
    assert!(
        managed_registry(&home).exists() == false,
        "present tools must not create a registry"
    );
    // --recommended with everything present has nothing to do.
    let (c2, out2) = run_with_env(
        &work,
        &home,
        Some(&fake_bin),
        ["tools", "install", "--recommended", "--yes", "--json"],
    );
    assert_eq!(c2, 0, "noop recommended runs succeed: {out2}");
    let v2: serde_json::Value = serde_json::from_str(&out2).unwrap();
    assert!(v2
        .get("installed")
        .and_then(|x| x.as_array())
        .unwrap()
        .is_empty());
}
