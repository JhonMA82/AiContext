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
                if (cand.contains('/') || cand.contains('.')) && !cand.contains(' ')
                    && !root.join(cand).exists() {
                        broken.push(cand.to_string());
                    }
            } else if !t.starts_with('-') && t.ends_with(':') {
                in_refs = false;
            }
        }
    }
    broken
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
            out.push(ok("config", format!("{} (schema {})", REPO_MANIFEST, c.schema)));
            Some(c)
        }
        Err(e) => {
            out.push(fail(
                "config",
                format!("{e:#}"),
                Some("aicontext init"),
            ));
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
                    format!("{} broken reference(s): {}", broken.len(), broken.join(", ")),
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

    // Stale tgrep index: a .tgrep/ dir without a tgrep binary is dead weight.
    // Positive condition first: only problems are reported, nothing when healthy.
    if root.join(".tgrep").exists() {
        if tools::detect_tool("tgrep").available {
            // healthy: index dir with its binary present, nothing to report
        } else {
            out.push(warn(
                "tgrep index",
                ".tgrep/ exists but tgrep is not installed",
                Some("install tgrep or delete .tgrep/"),
            ));
        }
    }

    // CodeGraph eligibility for large repos without it.
    if let Ok(report) = crate::scan::collect_scan(root) {
        if report.complexity.profile == "large" {
            if tools::detect_tool("codegraph").available {
                // healthy: large repo with graph analysis available
            } else {
                out.push(warn(
                    "codegraph",
                    format!(
                        "large repo ({}), codegraph not installed — impact queries degraded",
                        report.complexity.reason
                    ),
                    Some("see `aicontext tools plan`"),
                ));
            }
        }
    }

    out
}

pub fn cmd_doctor(json: bool) -> Result<i32> {
    let root = crate::scan::current_dir_root()?;
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
