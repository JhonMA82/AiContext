use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
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

fn fresh_home(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!(
        "aicontext-agent-{tag}-{}-{n}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    if base.exists() {
        std::fs::remove_dir_all(&base).expect("clean");
    }
    std::fs::create_dir_all(&base).expect("mkdir");
    base
}

fn work_dir(home: &Path, with_agents_md: bool) -> PathBuf {
    let dir = home.join("repo");
    std::fs::create_dir_all(&dir).expect("mkdir");
    if with_agents_md {
        std::fs::write(dir.join("AGENTS.md"), "# Test repo\n").unwrap();
    }
    dir
}

#[test]
fn install_is_idempotent_and_adds_pointer_once() {
    let home = fresh_home("install");
    let repo = work_dir(&home, true);
    let (c1, out1) = run_with_home(&repo, &home, ["agent", "install", "pi", "--json"]);
    assert_eq!(c1, 0, "install must succeed: {out1}");
    let v: serde_json::Value = serde_json::from_str(&out1).expect("must be JSON");
    assert_eq!(v["schema"], "aicontext/agent/v1");
    assert_eq!(v["changed"], true);

    let skill = home.join(".pi/agent/skills/aicontext-adopt/SKILL.md");
    assert!(skill.exists());
    let body = std::fs::read_to_string(&skill).unwrap();
    assert!(body.contains("# aicontext-adopt"));

    let (c2, out2) = run_with_home(&repo, &home, ["agent", "install", "pi", "--json"]);
    assert_eq!(c2, 0);
    let v2: serde_json::Value = serde_json::from_str(&out2).unwrap();
    assert_eq!(v2["changed"], false, "second install must be zero diff");
    let agents = std::fs::read_to_string(repo.join("AGENTS.md")).unwrap();
    assert_eq!(
        agents.matches(".engineering/PROJECT_STATE.md").count(),
        1,
        "pointer must not duplicate"
    );
}

#[test]
fn uninstall_removes_only_managed_files() {
    let home = fresh_home("uninstall");
    let repo = work_dir(&home, false);
    let (c, _) = run_with_home(&repo, &home, ["agent", "install", "pi"]);
    assert_eq!(c, 0);
    // Foreign file the tool must never delete.
    let foreign = home.join(".pi/agent/skills/aicontext-adopt/notes.txt");
    std::fs::write(&foreign, "mine").unwrap();

    let (cu, out) = run_with_home(&repo, &home, ["agent", "uninstall", "pi", "--json"]);
    assert_eq!(cu, 0, "uninstall must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let removed: Vec<&str> = v["removed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str())
        .collect();
    assert!(removed.contains(&"SKILL.md"));
    assert!(foreign.exists(), "foreign files must survive uninstall");
    assert!(!home
        .join(".pi/agent/skills/aicontext-adopt/SKILL.md")
        .exists());
}

#[test]
fn uninstall_without_install_is_noop() {
    let home = fresh_home("noop");
    let repo = work_dir(&home, false);
    let (c, out) = run_with_home(&repo, &home, ["agent", "uninstall", "pi", "--json"]);
    assert_eq!(
        c, 0,
        "uninstall without install must be a no-op success: {out}"
    );
}

#[test]
fn unknown_agent_fails_closed() {
    let home = fresh_home("unknown");
    let repo = work_dir(&home, false);
    let (c, out) = run_with_home(&repo, &home, ["agent", "install", "vscode", "--json"]);
    assert_eq!(c, 4, "unsupported agent must exit 4");
    assert!(
        out.contains("supported: pi, opencode"),
        "the error must name the supported agents: {out}"
    );
}

#[test]
fn opencode_install_targets_the_global_skills_dir_and_is_idempotent() {
    let home = fresh_home("opencode-install");
    let repo = work_dir(&home, true);
    let (c1, out1) = run_with_home(&repo, &home, ["agent", "install", "opencode", "--json"]);
    assert_eq!(c1, 0, "install must succeed: {out1}");
    let v: serde_json::Value = serde_json::from_str(&out1).expect("must be JSON");
    assert_eq!(v["schema"], "aicontext/agent/v1");
    assert_eq!(v["agent"], "opencode");
    assert_eq!(v["changed"], true);

    let skill = home.join(".config/opencode/skills/aicontext-adopt/SKILL.md");
    assert!(skill.exists(), "skill must land in the opencode skills dir");
    let body = std::fs::read_to_string(&skill).unwrap();
    assert!(body.contains("# aicontext-adopt"));
    assert!(
        !home.join(".pi/agent/skills/aicontext-adopt").exists(),
        "agents must not share a destination"
    );
    assert!(
        v["path"]
            .as_str()
            .unwrap_or("")
            .ends_with("opencode/skills/aicontext-adopt"),
        "reported path must be the opencode one: {v}"
    );

    let (c2, out2) = run_with_home(&repo, &home, ["agent", "install", "opencode", "--json"]);
    assert_eq!(c2, 0);
    let v2: serde_json::Value = serde_json::from_str(&out2).unwrap();
    assert_eq!(v2["changed"], false, "second install must be zero diff");
}

#[test]
fn opencode_install_honors_xdg_config_home() {
    let home = fresh_home("opencode-xdg");
    let xdg = home.join("xdg");
    let repo = work_dir(&home, false);
    let out = Command::new(bin())
        .args(["agent", "install", "opencode", "--json"])
        .current_dir(&repo)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &xdg)
        .output()
        .expect("run aicontext");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        xdg.join("opencode/skills/aicontext-adopt/SKILL.md")
            .exists(),
        "skill must follow XDG_CONFIG_HOME"
    );
    assert!(
        !home.join(".config/opencode").exists(),
        "the fallback dir must not be used when XDG_CONFIG_HOME is absolute"
    );
}

#[test]
fn opencode_uninstall_removes_only_managed_files() {
    let home = fresh_home("opencode-uninstall");
    let repo = work_dir(&home, false);
    let (c, _) = run_with_home(&repo, &home, ["agent", "install", "opencode"]);
    assert_eq!(c, 0);
    let skill = home.join(".config/opencode/skills/aicontext-adopt");
    let foreign = skill.join("notes.txt");
    std::fs::write(&foreign, "mine").unwrap();

    let (cu, out) = run_with_home(&repo, &home, ["agent", "uninstall", "opencode", "--json"]);
    assert_eq!(cu, 0, "uninstall must succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["agent"], "opencode");
    assert!(!skill.join("SKILL.md").exists(), "owned file must go");
    assert!(foreign.exists(), "foreign files must survive uninstall");
}
