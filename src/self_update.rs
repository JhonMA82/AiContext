use anyhow::{Context, Result};
use serde::Serialize;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::process::Command;

// JSON schema ids emitted by this module (documented here; schemas/ is
// intentionally not modified):
// - update (check and perform): "aicontext/self-update/v1"
// - uninstall: "aicontext/self-uninstall/v1"

const SELF_UPDATE_SCHEMA: &str = "aicontext/self-update/v1";
const SELF_UNINSTALL_SCHEMA: &str = "aicontext/self-uninstall/v1";

const INSTALLER_CMD: &str = "curl -LsSf https://github.com/JhonMA82/AiContext/releases/latest/download/aicontext-installer.sh | sh";
const GIT_URL: &str = "https://github.com/JhonMA82/AiContext";
const CARGO_INSTALL_CMD: &str =
    "cargo install --git https://github.com/JhonMA82/AiContext --locked";

fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn registry_url() -> String {
    // AICONTEXT_REGISTRY_URL exists so tests can point discovery at an
    // unroutable address and exercise the offline path without network.
    // Default is the GitHub Releases API: the crate is not published on
    // crates.io, so crates.io discovery always reports "not found".
    std::env::var("AICONTEXT_REGISTRY_URL").unwrap_or_else(|_| {
        "https://api.github.com/repos/JhonMA82/AiContext/releases/latest".to_string()
    })
}

/// Version discovery uses the GitHub Releases API, not the crates.io API:
/// the crate is not published on crates.io, and GitHub is where
/// cargo-dist publishes releases. The endpoint returns a single JSON
/// document with a stable `tag_name` field (`vX.Y.Z`), no
/// redirect-following or auth needed, and the same curl-based bounded call
/// pattern as crate::output::command_output (short max-time plus an outer
/// deadline, so an unreachable registry degrades instead of hanging).
/// An explicit User-Agent is sent because both the GitHub and crates.io
/// APIs reject bare requests (crates.io answers 403 without one).
fn fetch_latest_version() -> Option<String> {
    let url = registry_url();
    let agent = format!("aicontext/{}", current_version());
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "-f", "--max-time", "5", "-A", &agent, &url]);
    let out = crate::output::command_output(cmd, 10)?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    extract_version(&value)
}

fn extract_version(value: &serde_json::Value) -> Option<String> {
    let candidates = [
        value.pointer("/crate/max_version"),
        value.get("max_version"),
        value.get("version"),
        value.get("tag_name"),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Some(raw) = candidate.as_str() {
            let cleaned = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
            if !cleaned.is_empty() {
                return Some(cleaned.to_string());
            }
        }
    }
    None
}

fn numeric_prefix(part: &str) -> u64 {
    let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u64>().unwrap_or(0)
}

fn compare_versions(current: &str, latest: &str) -> Ordering {
    let mut cur = current.split('.');
    let mut lat = latest.split('.');
    loop {
        match (cur.next(), lat.next()) {
            (None, None) => return Ordering::Equal,
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (Some(a), Some(b)) => {
                let rank = numeric_prefix(a).cmp(&numeric_prefix(b));
                if rank == Ordering::Equal {
                    let tie = a.cmp(b);
                    if tie == Ordering::Equal {
                        continue;
                    }
                    return tie;
                }
                return rank;
            }
        }
    }
}

/// The crate is not published on crates.io, so updates come from the git
/// repository itself. When the latest release tag is known the install is
/// pinned to `v{latest}`; otherwise it tracks the default branch.
fn cargo_install_args(latest: Option<&str>) -> Vec<String> {
    match latest {
        Some(known) => vec![
            "install".to_string(),
            "--git".to_string(),
            GIT_URL.to_string(),
            "--tag".to_string(),
            format!("v{known}"),
            "--locked".to_string(),
        ],
        None => vec![
            "install".to_string(),
            "--git".to_string(),
            GIT_URL.to_string(),
            "--locked".to_string(),
        ],
    }
}

fn cargo_install_display(latest: Option<&str>) -> String {
    match latest {
        Some(known) => format!("cargo install --git {GIT_URL} --tag v{known} --locked"),
        None => CARGO_INSTALL_CMD.to_string(),
    }
}

fn cargo_present() -> bool {
    let mut cmd = Command::new("cargo");
    cmd.args(["--version"]);
    match crate::output::command_output(cmd, 10) {
        Some(out) => out.status.success(),
        None => false,
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("USERPROFILE").map(PathBuf::from))
        .context("neither HOME nor USERPROFILE is set")
}

fn managed_prefix() -> Result<PathBuf> {
    Ok(home_dir()?.join(".local").join("share").join("aicontext"))
}

fn cargo_bin_dir() -> Result<PathBuf> {
    match std::env::var("CARGO_HOME") {
        Ok(home) => Ok(PathBuf::from(home).join("bin")),
        Err(_) => Ok(home_dir()?.join(".cargo").join("bin")),
    }
}

fn path_inside(path: &Path, prefix: &Path) -> bool {
    path.starts_with(prefix)
}

/// cargo-dist installs leave no queryable receipt on disk, so
/// dist-managed is only claimed on explicit evidence: an env override or a
/// receipt file in the managed prefix. Anything else falls through to the
/// cargo path, and the binary itself is never overwritten in place.
fn dist_managed_evidence() -> bool {
    if std::env::var("AICONTEXT_DIST_MANAGED").map(|v| v == "1") == Ok(true) {
        return true;
    }
    match managed_prefix() {
        Ok(prefix) => prefix.join("install-receipt.json").exists(),
        Err(_) => false,
    }
}

fn binary_inside_managed_prefix(exe: &Path) -> bool {
    let prefixes: Vec<PathBuf> = [managed_prefix(), cargo_bin_dir()]
        .into_iter()
        .filter_map(|r| r.ok())
        .collect();
    prefixes.iter().any(|p| path_inside(exe, p))
}

#[derive(Debug, Clone, Serialize)]
struct UpdateJson<'a> {
    schema: &'a str,
    current: &'a str,
    latest: Option<&'a str>,
    up_to_date: Option<bool>,
    action: &'a str,
    method: Option<&'a str>,
    note: &'a str,
}

pub fn cmd_update(check: bool, yes: bool, json: bool) -> Result<i32> {
    let current = current_version();
    if check {
        return cmd_check(&current, json);
    }
    if !yes {
        let e = crate::output::AiError::new(
            "INVALID_USAGE",
            "refusing to update without --yes",
            Some("aicontext self update --yes"),
        );
        crate::output::print_error(&anyhow::anyhow!(e), json);
        return Ok(2);
    }
    let latest = fetch_latest_version();
    if let Some(ref known) = latest {
        if compare_versions(&current, known) == Ordering::Greater
            || compare_versions(&current, known) == Ordering::Equal
        {
            let note = format!("already up to date (current {current}, latest {known})");
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&UpdateJson {
                        schema: SELF_UPDATE_SCHEMA,
                        current: &current,
                        latest: latest.as_deref(),
                        up_to_date: Some(true),
                        action: "up-to-date",
                        method: None,
                        note: &note,
                    })?
                );
                return Ok(0);
            }
            println!("{note}");
            return Ok(0);
        }
    }
    if dist_managed_evidence() {
        let note = format!(
            "cargo-dist-managed install detected; rerun the official installer: {INSTALLER_CMD}"
        );
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&UpdateJson {
                    schema: SELF_UPDATE_SCHEMA,
                    current: &current,
                    latest: latest.as_deref(),
                    up_to_date: latest
                        .as_deref()
                        .map(|l| compare_versions(&current, l) != Ordering::Less),
                    action: "update",
                    method: Some("installer"),
                    note: &note,
                })?
            );
            return Ok(0);
        }
        println!("{note}");
        return Ok(0);
    }
    if cargo_present() {
        let display = cargo_install_display(latest.as_deref());
        let mut cmd = Command::new("cargo");
        cmd.args(cargo_install_args(latest.as_deref()));
        match crate::output::command_output(cmd, 300) {
            Some(out) if out.status.success() => {
                let note = format!("updated via `{display}`");
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&UpdateJson {
                            schema: SELF_UPDATE_SCHEMA,
                            current: &current,
                            latest: latest.as_deref(),
                            up_to_date: Some(true),
                            action: "update",
                            method: Some("cargo"),
                            note: &note,
                        })?
                    );
                    return Ok(0);
                }
                println!("{note}");
                return Ok(0);
            }
            _ => {
                let e = crate::output::AiError::new(
                    "TOOL_EXECUTION_FAILURE",
                    format!(
                        "`{display}` failed; latest known: {}",
                        latest.as_deref().unwrap_or("unknown (offline?)")
                    ),
                    Some(&display),
                );
                crate::output::print_error(&anyhow::anyhow!(e), json);
                return Ok(5);
            }
        }
    }
    let e = crate::output::AiError::new(
        "MISSING_REQUIRED_TOOL",
        "cargo is required for a managed update but was not found",
        Some(CARGO_INSTALL_CMD),
    );
    crate::output::print_error(&anyhow::anyhow!(e), json);
    Ok(3)
}

fn cmd_check(current: &str, json: bool) -> Result<i32> {
    match fetch_latest_version() {
        Some(latest) => {
            let newer = compare_versions(current, &latest) == Ordering::Less;
            let note = if newer {
                format!("update available (current {current}, latest {latest}); rerun with `--yes` to update")
            } else {
                format!("already up to date (current {current}, latest {latest})")
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&UpdateJson {
                        schema: SELF_UPDATE_SCHEMA,
                        current,
                        latest: Some(&latest),
                        up_to_date: Some(!newer),
                        action: "check",
                        method: None,
                        note: &note,
                    })?
                );
                return Ok(0);
            }
            println!("{note}");
            Ok(0)
        }
        None => {
            let note = format!(
                "registry unreachable (offline?); current version {current}, latest unknown — nothing was changed"
            );
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&UpdateJson {
                        schema: SELF_UPDATE_SCHEMA,
                        current,
                        latest: None,
                        up_to_date: None,
                        action: "check",
                        method: None,
                        note: &note,
                    })?
                );
                return Ok(0);
            }
            println!("{note}");
            Ok(0)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct UninstallJson<'a> {
    schema: &'a str,
    removed: &'a [String],
    left_alone: &'a [String],
    note: &'a str,
}

/// Uninstall scope (owned state only, never foreign files):
/// 1. The running binary, only when inside a managed prefix or cargo bin
///    (with --yes); an unidentified location is always left untouched.
/// 2. With --managed: tool files recorded in the ownership registry
///    (~/.local/share/aicontext/managed-tools.json), only when they sit
///    inside the managed prefix, plus the registry file itself.
/// 3. Agent skills with an AIContext ownership manifest
///    (~/.pi/agent/skills/aicontext-adopt/.aicontext-managed.json).
///
/// Never touched: foreign files, project manifests, .engineering/ dirs.
pub fn cmd_uninstall(managed: bool, yes: bool, json: bool) -> Result<i32> {
    if !yes {
        let e = crate::output::AiError::new(
            "INVALID_USAGE",
            "refusing to uninstall without --yes",
            Some("aicontext self uninstall --managed --yes"),
        );
        crate::output::print_error(&anyhow::anyhow!(e), json);
        return Ok(2);
    }
    let mut removed: Vec<String> = Vec::new();
    let mut left_alone: Vec<String> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        let display = exe.to_string_lossy().to_string();
        if binary_inside_managed_prefix(&exe) {
            match std::fs::remove_file(&exe) {
                Ok(()) => removed.push(format!("binary at {display}")),
                Err(e) => left_alone.push(format!("binary at {display} (remove failed: {e})")),
            }
        } else {
            left_alone.push(format!(
                "binary at {display} (outside managed prefix; left untouched)"
            ));
        }
    } else {
        left_alone.push("binary location unknown; left untouched".to_string());
    }

    if managed {
        remove_managed_tools(&mut removed, &mut left_alone)?;
    } else {
        left_alone.push("managed tools prefix (pass --managed to remove owned tools)".to_string());
    }

    remove_owned_skills(&mut removed, &mut left_alone)?;

    let note = "only AIContext-owned state was touched; foreign files, project manifests and .engineering/ dirs were left alone";
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&UninstallJson {
                schema: SELF_UNINSTALL_SCHEMA,
                removed: &removed,
                left_alone: &left_alone,
                note,
            })?
        );
        return Ok(0);
    }
    for r in &removed {
        println!("removed {r}");
    }
    for l in &left_alone {
        println!("left alone: {l}");
    }
    println!("{note}.");
    Ok(0)
}

fn remove_managed_tools(removed: &mut Vec<String>, left_alone: &mut Vec<String>) -> Result<()> {
    let prefix = managed_prefix()?;
    let registry_file = prefix.join("managed-tools.json");
    let registry = crate::tools_install::load_registry(&prefix);
    for entry in &registry.tools {
        let path = PathBuf::from(&entry.path);
        if path_inside(&path, &prefix) {
            if path.exists() {
                match std::fs::remove_file(&path) {
                    Ok(()) => {
                        removed.push(format!("managed tool {} at {}", entry.name, entry.path))
                    }
                    Err(e) => left_alone.push(format!(
                        "managed tool {} at {} (remove failed: {e})",
                        entry.name, entry.path
                    )),
                }
            } else {
                removed.push(format!(
                    "managed tool {} at {} (already absent)",
                    entry.name, entry.path
                ));
            }
        } else {
            left_alone.push(format!(
                "managed tool {} at {} (outside managed prefix; left untouched)",
                entry.name, entry.path
            ));
        }
    }
    if registry_file.exists() {
        std::fs::remove_file(&registry_file)?;
        removed.push(format!("ownership registry at {}", registry_file.display()));
    } else {
        left_alone.push("no ownership registry found".to_string());
    }
    Ok(())
}

fn remove_owned_skills(removed: &mut Vec<String>, left_alone: &mut Vec<String>) -> Result<()> {
    // Every agent this binary can install for; a new agent must be added to
    // `agent::SUPPORTED_AGENTS`, so the list stays the single source of truth.
    let mut found = false;
    for agent in crate::agent::SUPPORTED_AGENTS {
        let dir = crate::agent::skills_root(agent)?.join("aicontext-adopt");
        let manifest = dir.join(".aicontext-managed.json");
        if manifest.exists() {
            found = true;
            for owned in ["SKILL.md", ".aicontext-managed.json"] {
                let path = dir.join(owned);
                if path.exists() {
                    std::fs::remove_file(&path)?;
                    removed.push(format!("owned skill file {}", path.display()));
                }
            }
            match std::fs::remove_dir(&dir) {
                Ok(()) => removed.push(format!("skill dir {}", dir.display())),
                Err(_) => left_alone.push(format!(
                    "skill dir {} (foreign files remain; left untouched)",
                    dir.display()
                )),
            }
        } else if dir.join("SKILL.md").exists() {
            // Unmanaged install: report, never delete foreign content.
            found = true;
            left_alone.push(
                "skill dir exists without an AIContext ownership manifest; left untouched"
                    .to_string(),
            );
        }
    }
    if !found {
        left_alone.push("no owned agent skills found".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering_handles_missing_parts() {
        if compare_versions("0.1.0", "0.2.0") == Ordering::Less {
            // expected
        } else {
            panic!("0.1.0 must be older than 0.2.0");
        }
        assert_eq!(compare_versions("0.1.0", "0.1.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.2.0", "0.1.9"), Ordering::Greater);
        assert_eq!(compare_versions("0.1", "0.1.0"), Ordering::Less);
    }

    #[test]
    fn extract_version_prefers_crates_shape() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"crate":{"max_version":"1.4.2"}}"#).unwrap();
        assert_eq!(extract_version(&v).as_deref(), Some("1.4.2"));
        let tagged: serde_json::Value = serde_json::from_str(r#"{"tag_name":"v0.9.0"}"#).unwrap();
        assert_eq!(extract_version(&tagged).as_deref(), Some("0.9.0"));
        let empty: serde_json::Value = serde_json::from_str(r#"{"crate":{}}"#).unwrap();
        assert_eq!(extract_version(&empty), None);
    }

    #[test]
    fn extract_version_handles_github_release_shape() {
        let release: serde_json::Value = serde_json::from_str(
            r#"{"tag_name":"v0.4.0","name":"0.4.0","draft":false,"prerelease":false}"#,
        )
        .unwrap();
        assert_eq!(extract_version(&release).as_deref(), Some("0.4.0"));
    }

    #[test]
    fn cargo_install_uses_git_and_pins_tag() {
        let pinned = cargo_install_args(Some("0.4.0"));
        assert_eq!(
            pinned,
            vec!["install", "--git", GIT_URL, "--tag", "v0.4.0", "--locked"]
        );
        let display = cargo_install_display(Some("0.4.0"));
        assert!(
            display.contains("--git"),
            "display must show git: {display}"
        );
        assert!(
            display.contains("--tag v0.4.0"),
            "display must pin tag: {display}"
        );
        assert!(
            !display.contains("cargo install aicontext "),
            "must not use the unpublished crates.io name: {display}"
        );
        let unpinned = cargo_install_args(None);
        assert_eq!(unpinned, vec!["install", "--git", GIT_URL, "--locked"]);
    }
}
