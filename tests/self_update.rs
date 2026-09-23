use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
}

fn fresh_case(tag: &str) -> (PathBuf, PathBuf) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-self-{tag}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    let home = base.join("home");
    let work = base.join("work");
    std::fs::create_dir_all(&home).expect("mkdir home");
    std::fs::create_dir_all(&work).expect("mkdir work");
    (home, work)
}

fn git_init(dir: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "fixture", "--allow-empty"],
    ] {
        let st = Command::new("git")
            .args(&args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .expect("git");
        assert!(st.success(), "git {args:?} failed");
    }
}

fn run_self(home: &Path, work: &Path, extra_env: &[(&str, &str)], args: &[&str]) -> (i32, String) {
    let mut cmd = Command::new(bin());
    cmd.args(args).current_dir(work).env("HOME", home);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run aicontext");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(255), text)
}

#[test]
fn self_update_help_lists_update_and_uninstall() {
    let (home, work) = fresh_case("help");
    git_init(&work);
    let (c, out) = run_self(&home, &work, &[], &["self", "--help"]);
    assert_eq!(c, 0, "self --help must succeed: {out}");
    assert!(out.contains("update"), "help must list update: {out}");
    assert!(out.contains("uninstall"), "help must list uninstall: {out}");
}

#[test]
fn self_update_check_offline_degrades_without_changes() {
    let (home, work) = fresh_case("check-offline");
    git_init(&work);
    // Unroutable loopback port: connection refused instantly, no network used.
    let (c, out) = run_self(
        &home,
        &work,
        &[("AICONTEXT_REGISTRY_URL", "http://127.0.0.1:9/")],
        &["self", "update", "--check"],
    );
    assert_eq!(
        c, 0,
        "offline --check must exit 0, never hang or fail: {out}"
    );
    assert!(
        out.contains("registry unreachable"),
        "must report unreachable registry: {out}"
    );
    assert!(
        out.contains("nothing was changed"),
        "must state nothing changed: {out}"
    );
    assert!(
        out.contains(env!("CARGO_PKG_VERSION")),
        "must report the current version: {out}"
    );
    let registry = home.join(".local/share/aicontext/managed-tools.json");
    if registry.exists() {
        panic!("--check must not create any state");
    }
}

#[test]
fn self_update_check_offline_json_schema() {
    let (home, work) = fresh_case("check-json");
    git_init(&work);
    let (c, out) = run_self(
        &home,
        &work,
        &[("AICONTEXT_REGISTRY_URL", "http://127.0.0.1:9/")],
        &["self", "update", "--check", "--json"],
    );
    assert_eq!(c, 0, "offline --check --json must exit 0: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("--json must be JSON");
    assert_eq!(v["schema"], "aicontext/self-update/v1");
    assert_eq!(v["current"], env!("CARGO_PKG_VERSION"));
    assert!(v.get("latest").is_some() && v["latest"].is_null());
    assert_eq!(v["action"], "check");
}

#[test]
fn self_update_refuses_without_yes() {
    let (home, work) = fresh_case("update-refuse");
    git_init(&work);
    let (c, out) = run_self(&home, &work, &[], &["self", "update"]);
    assert_eq!(c, 2, "update without --yes must exit 2: {out}");
    assert!(
        out.contains("aicontext self update --yes"),
        "must print the exact remediation: {out}"
    );
}

#[test]
fn self_update_uninstall_refuses_without_yes() {
    let (home, work) = fresh_case("uninstall-refuse");
    git_init(&work);
    let (c, out) = run_self(&home, &work, &[], &["self", "uninstall", "--managed"]);
    assert_eq!(c, 2, "uninstall without --yes must exit 2: {out}");
    assert!(
        out.contains("aicontext self uninstall --managed --yes"),
        "must print the exact remediation: {out}"
    );
}

#[test]
fn self_update_uninstall_managed_removes_only_owned_state() {
    let (home, work) = fresh_case("uninstall-scope");
    git_init(&work);

    // Owned tool recorded in the ownership registry (inside managed prefix).
    let prefix = home.join(".local/share/aicontext");
    let owned = prefix.join("bin/owned-tool");
    std::fs::create_dir_all(owned.parent().unwrap()).expect("mkdir bin");
    std::fs::write(&owned, "owned").expect("write owned tool");
    let registry_body = serde_json::json!({
        "tools": [{
            "name": "owned-tool",
            "version": "1.0",
            "method": "test",
            "path": owned.to_string_lossy(),
            "installed_at": 1,
            "managed_by": "managed"
        }]
    });
    std::fs::write(
        prefix.join("managed-tools.json"),
        serde_json::to_string_pretty(&registry_body).unwrap(),
    )
    .expect("write registry");

    // Foreign file inside the prefix but absent from the registry: untouchable.
    let foreign_in_prefix = prefix.join("bin/foreign-tool");
    std::fs::write(&foreign_in_prefix, "foreign").expect("write foreign tool");

    // Owned agent skill (manifest present) plus a foreign file beside it.
    let skill_dir = home.join(".pi/agent/skills/aicontext-adopt");
    std::fs::create_dir_all(&skill_dir).expect("mkdir skill");
    std::fs::write(skill_dir.join("SKILL.md"), "owned skill").expect("write skill");
    std::fs::write(skill_dir.join(".aicontext-managed.json"), "{}").expect("write manifest");
    std::fs::write(skill_dir.join("keep.me"), "foreign").expect("write foreign skill file");

    // The same ownership claim in the opencode skills dir. Nothing foreign
    // lives there, so the whole dir must be removed with its files.
    let oc_skill_dir = home.join(".config/opencode/skills/aicontext-adopt");
    std::fs::create_dir_all(&oc_skill_dir).expect("mkdir opencode skill");
    std::fs::write(oc_skill_dir.join("SKILL.md"), "owned skill").expect("write skill");
    std::fs::write(oc_skill_dir.join(".aicontext-managed.json"), "{}").expect("write manifest");

    // Project files that uninstall must never touch.
    std::fs::create_dir_all(work.join(".engineering")).expect("mkdir engineering");
    std::fs::write(work.join(".engineering/sentinel"), "keep").expect("write sentinel");
    std::fs::write(work.join("package.json"), "{\"name\":\"proj\"}").expect("write manifest");
    std::fs::write(work.join("foreign.txt"), "foreign").expect("write foreign");

    let (c, out) = run_self(
        &home,
        &work,
        &[],
        &["self", "uninstall", "--managed", "--yes", "--json"],
    );
    assert_eq!(c, 0, "owned-only uninstall must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("--json must be JSON");
    assert_eq!(v["schema"], "aicontext/self-uninstall/v1");

    // Owned state is gone.
    for (path, what) in [
        (owned.clone(), "owned tool must be removed"),
        (
            prefix.join("managed-tools.json"),
            "ownership registry must be removed",
        ),
        (skill_dir.join("SKILL.md"), "owned skill must be removed"),
        (
            skill_dir.join(".aicontext-managed.json"),
            "ownership manifest must be removed",
        ),
        (
            oc_skill_dir.join("SKILL.md"),
            "owned opencode skill must be removed",
        ),
        (
            oc_skill_dir.join(".aicontext-managed.json"),
            "opencode ownership manifest must be removed",
        ),
    ] {
        if path.exists() {
            panic!("{what}: {}", path.display());
        }
    }
    assert!(
        !oc_skill_dir.exists(),
        "an opencode skill dir with no foreign files must be removed entirely"
    );

    // Everything foreign is left alone.
    assert!(
        foreign_in_prefix.exists(),
        "foreign prefix file must remain"
    );
    assert!(
        skill_dir.join("keep.me").exists(),
        "foreign skill-dir file must remain"
    );
    assert!(
        std::fs::read_to_string(work.join("package.json"))
            .unwrap()
            .contains("proj"),
        "project manifest must be untouched"
    );
    assert!(
        work.join("foreign.txt").exists(),
        "foreign file must remain"
    );
    assert!(
        work.join(".engineering/sentinel").exists(),
        ".engineering/ must never be touched"
    );
    assert!(
        bin().exists(),
        "test binary (outside managed prefix) must remain"
    );
}
