use anyhow::Result;
use serde::Serialize;
use std::path::Path;

use crate::config::{CONSISTENCY, PATTERNS, PROJECT_STATE, REPO_MANIFEST};
use crate::tools;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub name: String,
    pub status: Status,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

fn ok(name: &str, detail: impl Into<String>) -> Diagnostic {
    Diagnostic {
        name: name.to_string(),
        status: Status::Ok,
        detail: detail.into(),
        remediation: None,
    }
}

fn warn(name: &str, detail: impl Into<String>, remediation: Option<&str>) -> Diagnostic {
    Diagnostic {
        name: name.to_string(),
        status: Status::Warn,
        detail: detail.into(),
        remediation: remediation.map(|s| s.to_string()),
    }
}

fn fail(name: &str, detail: impl Into<String>, remediation: Option<&str>) -> Diagnostic {
    Diagnostic {
        name: name.to_string(),
        status: Status::Fail,
        detail: detail.into(),
        remediation: remediation.map(|s| s.to_string()),
    }
}

fn broken_path_refs(root: &Path, patterns_path: &Path) -> Vec<String> {
    let text = match std::fs::read_to_string(patterns_path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut broken = Vec::new();
    let mut in_refs = false;
    for line in text.lines() {
        let t = line.trim();
        if t.eq_ignore_ascii_case("reference:") || t.eq_ignore_ascii_case("references:") {
            in_refs = true;
            continue;
        }
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if in_refs {
            if let Some(rest) = t.strip_prefix("- ") {
                let cand = rest.trim();
                // Path-looking entries only: contain / or . and no spaces.
                if (cand.contains('/') || cand.contains('.'))
                    && !cand.contains(' ')
                    && !root.join(cand).exists()
                {
                    broken.push(cand.to_string());
                }
            } else if !t.starts_with('-') && t.ends_with(':') {
                in_refs = false;
            }
        }
    }
    broken
}

/// Parse `tgrep status` into a one-line summary. None when status fails,
/// times out, or prints anything but a real index report — notably the
/// exit-0 "No index found" message, which must degrade to the corrupt-index
/// warning instead of a healthy Ok with `?` placeholders.
fn tgrep_status_summary(root: &Path) -> Option<String> {
    let mut cmd = std::process::Command::new("tgrep");
    cmd.args(["status", "."]).current_dir(root);
    let out = crate::output::command_output(cmd, 15)?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut files: Option<String> = None;
    let mut server: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Files:") {
            files = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("Server:") {
            server = Some(rest.trim().to_string());
        }
    }
    Some(format!("{} files indexed, server: {}", files?, server?))
}

pub fn run_doctor(root: &Path) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    out.push(ok(
        "cli",
        format!("aicontext {}", env!("CARGO_PKG_VERSION")),
    ));

    let git = tools::detect_tool("git");
    if git.available {
        out.push(ok(
            "git",
            git.version.unwrap_or_else(|| "available".to_string()),
        ));
    } else {
        out.push(fail(
            "git",
            "git binary not found; Git is the source of truth for scan",
            Some("install git and re-run"),
        ));
    }

    let cfg = match crate::config::RepoConfig::load(root) {
        Ok(c) => {
            out.push(ok(
                "config",
                format!("{} (schema {})", REPO_MANIFEST, c.schema),
            ));
            Some(c)
        }
        Err(e) => {
            out.push(fail("config", format!("{e:#}"), Some("aicontext init")));
            None
        }
    };

    if let Some(c) = &cfg {
        let state_path = c.state_path(root);
        match std::fs::read_to_string(&state_path) {
            Ok(text) => {
                let has_markers = text.contains(crate::state::GEN_START)
                    && text.contains(crate::state::GEN_END)
                    && text.contains(crate::state::CUR_START)
                    && text.contains(crate::state::CUR_END);
                if has_markers {
                    out.push(ok("state markers", c.state.project_state.clone()));
                } else {
                    out.push(fail(
                        "state markers",
                        "PROJECT_STATE.md lacks generated/curated markers",
                        Some("aicontext init"),
                    ));
                }
            }
            Err(_) => out.push(fail(
                "state markers",
                format!("missing {}", c.state.project_state),
                Some("aicontext init"),
            )),
        }

        let patterns_path = root.join(&c.state.patterns);
        if patterns_path.exists() {
            let broken = broken_path_refs(root, &patterns_path);
            if broken.is_empty() {
                out.push(ok("patterns refs", c.state.patterns.clone()));
            } else {
                out.push(warn(
                    "patterns refs",
                    format!(
                        "{} broken reference(s): {}",
                        broken.len(),
                        broken.join(", ")
                    ),
                    Some("fix paths in PATTERNS.md or re-run adoption"),
                ));
            }
        } else {
            out.push(warn(
                "patterns refs",
                format!("missing {}", c.state.patterns),
                Some("aicontext init"),
            ));
        }

        let consistency_path = root.join(&c.consistency.manifest);
        match std::fs::read_to_string(&consistency_path) {
            Ok(text) => match serde_yaml::from_str::<serde_yaml::Value>(&text) {
                Ok(v)
                    if v.get("schema").and_then(|s| s.as_str())
                        == Some("aicontext/consistency/v1") =>
                {
                    out.push(ok("consistency", c.consistency.manifest.clone()))
                }
                _ => out.push(fail(
                    "consistency",
                    format!("{CONSISTENCY} is not aicontext/consistency/v1"),
                    Some("aicontext init"),
                )),
            },
            Err(_) => out.push(fail(
                "consistency",
                format!("missing {CONSISTENCY}"),
                Some("aicontext init"),
            )),
        }
    }

    let sg = tools::detect_tool("ast-grep");
    if sg.available {
        out.push(ok(
            "ast-grep",
            sg.version.unwrap_or_else(|| "available".to_string()),
        ));
    } else {
        out.push(warn(
            "ast-grep",
            "absent — structural search and rules degraded",
            Some("see `aicontext tools plan`"),
        ));
    }

    // Installed skill vs this binary (WU7): the skill text is embedded,
    // so an on-disk SKILL.md that differs from it is either stale (an
    // older binary installed it) or hand-modified. Both warn with
    // reinstall as remediation; a missing install warns too. The skill is
    // opt-in, so this diagnostic never fails.
    match crate::agent::installed_skill() {
        None => out.push(warn(
            "skill",
            "aicontext-adopt is not installed for pi",
            Some("aicontext agent install pi"),
        )),
        Some(installed) if installed.body == crate::agent::SKILL_BODY => out.push(ok(
            "skill",
            format!(
                "aicontext-adopt up to date (v{})",
                env!("CARGO_PKG_VERSION")
            ),
        )),
        Some(installed) => out.push(warn(
            "skill",
            format!(
                "installed skill differs from this binary (installed: {}, binary: v{})",
                installed.version.as_deref().unwrap_or("unmanaged"),
                env!("CARGO_PKG_VERSION")
            ),
            Some("aicontext agent install pi"),
        )),
    }

    // RTK name-collision trap (§41 example): only meaningful when an `rtk`
    // binary exists.
    if tools::detect_tool("rtk").available {
        match tools::rtk_health() {
            None => out.push(ok("rtk", "Rust Token Killer healthy (`rtk gain` works)")),
            Some(detail) => out.push(fail(
                "rtk",
                detail,
                Some("install Rust Token Killer from rtk-ai/rtk"),
            )),
        }
    }

    // Stale tgrep index: a .tgrep/ dir without a tgrep binary is dead weight;
    // with the binary present, `tgrep status` is authoritative.
    // Positive condition first: only problems are reported, nothing when healthy.
    if root.join(".tgrep").exists() {
        if tools::detect_tool("tgrep").available {
            match tgrep_status_summary(root) {
                Some(summary) => out.push(ok("tgrep index", summary)),
                None => out.push(warn(
                    "tgrep index",
                    ".tgrep/ exists but `tgrep status` fails; the index may be corrupt",
                    Some("rebuild with `tgrep index .`"),
                )),
            }
        } else {
            out.push(warn(
                "tgrep index",
                ".tgrep/ exists but tgrep is not installed",
                Some("install tgrep or delete .tgrep/"),
            ));
        }
    }

    // Graph backend eligibility for large repos. Any usable graph backend
    // (codebase-memory-mcp preferred, codegraph as fallback) serves impact
    // queries; without one, impact degrades to text search. Advisory only:
    // `doctor` never fails or blocks because a graph is missing.
    if let Ok(report) = crate::scan::collect_scan(root) {
        if report.complexity.profile == "large" {
            let cfg = crate::config::RepoConfig::load(root).ok();
            let usable: Vec<&str> = ["codebase-memory-mcp", "codegraph"]
                .into_iter()
                .filter(|tool| {
                    matches!(
                        tools::tool_state(cfg.as_ref(), tool),
                        tools::ToolState::Usable
                    )
                })
                .collect();
            if usable.is_empty() {
                out.push(warn(
                    "graph backend",
                    format!(
                        "large repo ({}), no graph backend — impact queries degraded",
                        report.complexity.reason
                    ),
                    Some("run `aicontext tools plan`"),
                ));
            } else {
                out.push(ok(
                    "graph backend",
                    format!(
                        "large repo ({}): graph backend available ({})",
                        report.complexity.reason,
                        usable.join(", ")
                    ),
                ));
            }
        }
    }

    // Engineering origin (advisory only): `eng doctor` owns materialization
    // invariants; `aicontext check` owns the drift gate. Doctor only reports
    // which mode this repo is in. Standalone repos emit no diagnostic.
    match crate::engineering::detect(root) {
        crate::engineering::EngineeringDetection::Absent => {}
        crate::engineering::EngineeringDetection::Supported(info) => out.push(ok(
            "engineering",
            format!(
                "engineering-managed (recipe {}, {} surface(s) via .engineering/project-map.json)",
                info.recipe,
                info.surfaces.len()
            ),
        )),
        crate::engineering::EngineeringDetection::Unsupported { reason, .. } => out.push(warn(
            "engineering",
            format!("engineering contracts present but unsupported: {reason}"),
            Some("upgrade aicontext or regenerate with a compatible Engineering Platform; see `aicontext check`"),
        )),
    }

    out
}

pub fn cmd_doctor(json: bool) -> Result<i32> {
    let root = crate::scan::resolve_project_root()?;
    let diags = run_doctor(&root);
    let failed = diags.iter().any(|d| d.status == Status::Fail);
    let code = if failed { 1 } else { 0 };
    if json {
        #[derive(Serialize)]
        struct DoctorOut<'a> {
            schema: &'a str,
            healthy: bool,
            diagnostics: &'a [Diagnostic],
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&DoctorOut {
                schema: "aicontext/doctor/v1",
                healthy: !failed,
                diagnostics: &diags,
            })?
        );
        return Ok(code);
    }
    for d in &diags {
        let (mark, label) = match d.status {
            Status::Ok => ("✓", "ok"),
            Status::Warn => ("!", "warning"),
            Status::Fail => ("✗", "problem"),
        };
        println!("{mark} {} [{label}] — {}", d.name, d.detail);
        if let Some(r) = &d.remediation {
            println!("    remediation: {r}");
        }
    }
    let _ = (PATTERNS, PROJECT_STATE);
    Ok(code)
}
