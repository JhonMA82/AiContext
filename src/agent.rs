use anyhow::{Context, Result};
use serde::Serialize;
use std::path::PathBuf;

const SKILL_NAME: &str = "aicontext-adopt";
pub(crate) const SKILL_BODY: &str = include_str!("../skills/aicontext-adopt/SKILL.md");
const MANAGED_MANIFEST: &str = ".aicontext-managed.json";

/// Legacy, unmarked AGENTS.md pointer appended by `agent install`. The
/// marked context block in `state.rs` wraps exactly this body, and the
/// router must report — never rewrite — a file that already contains it.
pub(crate) const AGENTS_POINTER: &str = "## Repository context\n\nBefore broad repository exploration, read:\n- `.engineering/PROJECT_STATE.md`\n- `.engineering/PATTERNS.md`\n\nPrefer `aicontext search` and declared references before broad scanning.\n\nBefore declaring implementation complete, run:\n`aicontext check`\n";

/// True when the pre-marker pointer text is present without the marked
/// context block. `init`/`sync` must not rewrite or duplicate it: they
/// report the legacy pointer and place the routing block deterministically.
pub(crate) fn legacy_pointer_present(text: &str) -> bool {
    text.contains("## Repository context")
        && text.contains(".engineering/PROJECT_STATE.md")
        && !text.contains(crate::state::CONTEXT_START)
}

fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("USERPROFILE").map(PathBuf::from))
        .context("neither HOME nor USERPROFILE is set")
}

fn skills_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".pi").join("agent").join("skills"))
}

/// Installed skill state for the doctor skew gate: the on-disk `SKILL.md`
/// body plus the managing binary version from the ownership manifest
/// (`None` when the install is unmanaged). A missing file or directory
/// reads as `None` (not installed), never an error.
pub(crate) struct InstalledSkill {
    pub body: String,
    pub version: Option<String>,
}

pub(crate) fn installed_skill() -> Option<InstalledSkill> {
    let body = std::fs::read_to_string(skill_dir().ok()?.join("SKILL.md")).ok()?;
    let version = skill_dir()
        .ok()
        .and_then(|dir| std::fs::read_to_string(dir.join(MANAGED_MANIFEST)).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| {
            v.get("version")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
        });
    Some(InstalledSkill { body, version })
}

fn skill_dir() -> Result<PathBuf> {
    Ok(skills_dir()?.join(SKILL_NAME))
}

#[derive(Serialize)]
struct ManagedManifest<'a> {
    managed_by: &'a str,
    version: &'a str,
    files: Vec<&'a str>,
}

fn write_if_different(path: &std::path::Path, content: &str) -> Result<bool> {
    let current = std::fs::read_to_string(path).ok();
    if current.as_deref() == Some(content) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

/// Append the minimal AGENTS.md pointer. Returns true when it changed something.
fn ensure_agents_pointer(repo: &std::path::Path) -> Result<(bool, String)> {
    let path = repo.join("AGENTS.md");
    if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        if text.contains(".engineering/PROJECT_STATE.md") {
            return Ok((false, "AGENTS.md pointer already present".to_string()));
        }
        let mut text = text;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push('\n');
        text.push_str(AGENTS_POINTER);
        std::fs::write(&path, text)?;
        return Ok((true, "AGENTS.md pointer appended".to_string()));
    }
    Ok((
        false,
        "no AGENTS.md in repo; pointer not added (create one to opt in)".to_string(),
    ))
}

fn check_agent(agent: &str) -> Result<()> {
    if agent == "pi" {
        return Ok(());
    }
    Err(crate::output::AiError::new(
        "UNSUPPORTED_AGENT",
        format!("agent '{agent}' is not supported in v0.1 (supported: pi)"),
        None,
    )
    .into())
}

pub fn cmd_install(agent: String, json: bool) -> Result<i32> {
    if let Err(e) = check_agent(&agent) {
        if json {
            crate::output::print_error(&e, true);
        } else {
            eprintln!("error: {e:#}");
        }
        return Ok(4);
    }
    let dir = skill_dir()?;
    let skill_path = dir.join("SKILL.md");
    let manifest_path = dir.join(MANAGED_MANIFEST);
    let skill_changed = write_if_different(&skill_path, SKILL_BODY)?;
    let manifest = ManagedManifest {
        managed_by: "aicontext",
        version: env!("CARGO_PKG_VERSION"),
        files: vec!["SKILL.md", MANAGED_MANIFEST],
    };
    let manifest_changed =
        write_if_different(&manifest_path, &serde_json::to_string_pretty(&manifest)?)?;
    let repo = std::env::current_dir()?;
    let (pointer_changed, pointer_note) = ensure_agents_pointer(&repo)?;
    let changed = skill_changed || manifest_changed || pointer_changed;

    if json {
        #[derive(Serialize)]
        struct AgentOut<'a> {
            schema: &'a str,
            action: &'a str,
            agent: &'a str,
            path: &'a str,
            changed: bool,
            note: &'a str,
        }
        let path = dir.to_string_lossy().into_owned();
        println!(
            "{}",
            serde_json::to_string_pretty(&AgentOut {
                schema: "aicontext/agent/v1",
                action: "install",
                agent: &agent,
                path: &path,
                changed,
                note: &pointer_note,
            })?
        );
        return Ok(0);
    }
    println!("Installed skill `{SKILL_NAME}` for Pi at {}", dir.display());
    println!("Invoke manually with /{SKILL_NAME}. It never auto-runs.");
    println!("Note: {pointer_note}");
    if !changed {
        println!("Already up to date (idempotent, zero diff).");
    }
    Ok(0)
}

pub fn cmd_uninstall(agent: String, json: bool) -> Result<i32> {
    if let Err(e) = check_agent(&agent) {
        if json {
            crate::output::print_error(&e, true);
        } else {
            eprintln!("error: {e:#}");
        }
        return Ok(4);
    }
    let dir = skill_dir()?;
    let manifest_path = dir.join(MANAGED_MANIFEST);
    let mut removed: Vec<String> = Vec::new();
    let mut note = String::from("nothing to remove");
    if manifest_path.exists() {
        // Remove exactly the files this tool owns; never touch foreign files.
        for owned in ["SKILL.md", MANAGED_MANIFEST] {
            let p = dir.join(owned);
            if p.exists() {
                std::fs::remove_file(&p)?;
                removed.push(owned.to_string());
            }
        }
        // Remove the dir only when nothing foreign remains.
        match std::fs::remove_dir(&dir) {
            Ok(()) => note = format!("removed {}", dir.display()),
            Err(_) => note = format!("kept {} (foreign files remain)", dir.display()),
        }
    } else if dir.join("SKILL.md").exists() {
        note = "skill dir exists without an AIContext ownership manifest; left untouched".into();
    }
    // The AGENTS.md pointer lives in user-owned files: report, never edit blindly.
    let repo = std::env::current_dir()?;
    let agents_path = repo.join("AGENTS.md");
    let pointer_note = if agents_path.exists()
        && std::fs::read_to_string(&agents_path)
            .map(|t| t.contains(".engineering/PROJECT_STATE.md"))
            .unwrap_or(false)
    {
        "AGENTS.md pointer left in place; remove it manually if desired"
    } else {
        "no AGENTS.md pointer found"
    };

    if json {
        #[derive(Serialize)]
        struct AgentOut<'a> {
            schema: &'a str,
            action: &'a str,
            agent: &'a str,
            removed: &'a [String],
            note: &'a str,
            agents_pointer: &'a str,
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&AgentOut {
                schema: "aicontext/agent/v1",
                action: "uninstall",
                agent: &agent,
                removed: &removed,
                note: &note,
                agents_pointer: pointer_note,
            })?
        );
        return Ok(0);
    }
    if removed.is_empty() {
        println!("{note}.");
    } else {
        println!("Removed managed files: {}.", removed.join(", "));
        println!("{note}.");
    }
    println!("{pointer_note}.");
    Ok(0)
}
