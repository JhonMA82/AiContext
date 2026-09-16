//! Project adapters (spec P4): knip, dependency-cruiser, lychee, zizmor.
//!
//! Conceptual interface per spec §63: detect -> applicability,
//! plan -> recommendation (`tools plan`), check -> findings (this module).
//! Findings are evidence only by default: they never fail `check` (spec
//! §29 — an optional tool never blocks a repo without explicit
//! configuration). Declaring `adapters.<tool>.policy = "required"` in
//! consistency.yml opts that adapter into a real gate: only a clean
//! executed run passes (see [`KNOWN_ADAPTERS`]).
//!
//! All runners are read-only and offline-first:
//! - knip runs project-local (`knip --reporter json --no-exit-code`);
//! - dependency-cruiser runs only when a config exists, never invents rules;
//! - lychee runs with `--offline` (local files only, no network);
//! - zizmor runs with `--offline` (no token, no fetches).
//! A missing binary or config degrades to an informational note, never an error.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Outcome of one adapter evaluation. Only applicable adapters are returned
/// by [`run_all`]; non-applicable ones produce no finding at all.
#[derive(Debug, Clone)]
pub struct AdapterOutcome {
    pub tool: &'static str,
    /// Machine state: ok | issues | missing | unconfigured | error.
    pub state: String,
    pub detail: String,
}

fn truncate(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

fn is_js_ts(root: &Path) -> bool {
    root.join("package.json").exists()
}

fn has_workflows(root: &Path) -> bool {
    root.join(".github/workflows").exists()
}

/// Config filenames dependency-cruiser discovers by default (JS flavor).
const DEP_CRUISE_CONFIGS: &[&str] = &[
    ".dependency-cruiser.json",
    ".dependency-cruiser.js",
    ".dependency-cruiser.cjs",
    ".dependency-cruiser.mjs",
    "dependency-cruiser.config.js",
];

/// Config filename when one exists (for plan wording and the `--config` flag).
pub fn depcruiser_config_name(root: &Path) -> Option<String> {
    DEP_CRUISE_CONFIGS
        .iter()
        .find(|name| root.join(name).exists())
        .map(|s| s.to_string())
}

/// Lychee applies to docs-heavy repos, mirroring the `tools plan` condition.
fn lychee_applicable(root: &Path, md_files: usize) -> bool {
    root.join("docs").exists() || md_files >= 10
}

fn md_file_count(report: &crate::scan::ScanReport) -> usize {
    report
        .languages
        .iter()
        .filter(|l| l.language == "markdown")
        .map(|l| l.files)
        .sum()
}

/// Adapter names accepted in the `adapters` table of consistency.yml.
/// Unknown names fail `check` closed (typo protection).
pub const KNOWN_ADAPTERS: &[&str] = &["knip", "dependency-cruiser", "lychee", "zizmor"];

/// Run every adapter that applies to this repo. Never fails: each outcome
/// carries its own state (`ok`, `issues`, `missing`, `unconfigured`, `error`).
pub fn run_all(root: &Path) -> Vec<AdapterOutcome> {
    let report = crate::scan::collect_scan(root).ok();
    let md_files = report.as_ref().map(md_file_count).unwrap_or(0);
    let mut out = Vec::new();
    if is_js_ts(root) {
        out.push(run_knip(root));
        out.push(run_depcruiser(root));
    }
    if lychee_applicable(root, md_files) {
        out.push(run_lychee(root));
    }
    if has_workflows(root) {
        out.push(run_zizmor(root));
    }
    out
}

/// Shared tail: stdout parsed as JSON wins over the exit code (knip exits 1
/// on findings, zizmor may too); exit 0 with unparsable output means clean.
fn classify(
    tool: &'static str,
    spawned: std::io::Result<std::process::Output>,
    parse: impl FnOnce(&serde_json::Value) -> Option<(u64, String)>,
    install_hint: &str,
) -> AdapterOutcome {
    let Ok(out) = spawned else {
        return AdapterOutcome {
            tool,
            state: "missing".to_string(),
            detail: format!("{tool} absent — install: {install_hint} (evidence unavailable)"),
        };
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&stdout) {
        if let Some((issues, summary)) = parse(&v) {
            let state = if issues == 0 { "ok" } else { "issues" };
            return AdapterOutcome {
                tool,
                state: state.to_string(),
                detail: format!("{tool}: {summary}"),
            };
        }
    }
    if out.status.success() {
        AdapterOutcome {
            tool,
            state: "ok".to_string(),
            detail: format!("{tool}: ran clean (no machine-readable issues)"),
        }
    } else {
        let stderr = truncate(String::from_utf8_lossy(&out.stderr).trim(), 200);
        AdapterOutcome {
            tool,
            state: "error".to_string(),
            detail: format!("{tool}: run failed ({stderr}); evidence unavailable — {install_hint}"),
        }
    }
}

/// Knip issue types (knip.dev/reference/cli). Only these keys are counted so
/// reporter metadata never inflates the total.
const KNIP_ISSUE_TYPES: &[&str] = &[
    "files",
    "dependencies",
    "unlisted",
    "binaries",
    "unresolved",
    "exports",
    "nsExports",
    "types",
    "nsTypes",
    "enumMembers",
    "namespaceMembers",
    "duplicates",
    "cycles",
    "catalog",
    "catalogReferences",
];

fn run_knip(root: &Path) -> AdapterOutcome {
    if crate::tools::detect_tool("knip").available {
        // Present: run below.
    } else {
        return AdapterOutcome {
            tool: "knip",
            state: "missing".to_string(),
            detail: "knip absent — install: npm install --save-dev knip (project-local; evidence unavailable)".to_string(),
        };
    }
    let spawned = Command::new("knip")
        .args(["--reporter", "json", "--no-exit-code"])
        .current_dir(root)
        .output();
    classify(
        "knip",
        spawned,
        |v| {
            let obj = v.as_object()?;
            let mut total = 0u64;
            let mut parts = Vec::new();
            for key in KNIP_ISSUE_TYPES {
                let n = match obj.get(*key) {
                    Some(serde_json::Value::Array(a)) => a.len() as u64,
                    Some(serde_json::Value::Object(o)) => o.len() as u64,
                    _ => 0,
                };
                if n > 0 {
                    total += n;
                    parts.push(format!("{key} {n}"));
                }
            }
            let summary = if total == 0 {
                "no unused files, dependencies or exports".to_string()
            } else {
                format!("{} issue(s): {}", total, truncate(&parts.join(", "), 200))
            };
            Some((total, summary))
        },
        "npm install --save-dev knip",
    )
}

fn run_depcruiser(root: &Path) -> AdapterOutcome {
    const TOOL: &str = "dependency-cruiser";
    let Some(config) = depcruiser_config_name(root) else {
        return AdapterOutcome {
            tool: TOOL,
            state: "unconfigured".to_string(),
            detail: "dependency-cruiser: no boundaries config — formalize only if worth it (.dependency-cruiser.json)".to_string(),
        };
    };
    let binary = if crate::tools::detect_tool("depcruise").available {
        "depcruise"
    } else if crate::tools::detect_tool("dependency-cruiser").available {
        "dependency-cruiser"
    } else {
        return AdapterOutcome {
            tool: TOOL,
            state: "missing".to_string(),
            detail: "dependency-cruiser absent — install: npm install --save-dev dependency-cruiser (project-local; evidence unavailable)".to_string(),
        };
    };
    let spawned = Command::new(binary)
        .args(["--output-type", "json", "--config", &config, "."])
        .current_dir(root)
        .output();
    classify(
        TOOL,
        spawned,
        |v| {
            let summary = v.get("summary")?;
            // Prefer the violations array; fall back to severity counters.
            if let Some(arr) = summary.get("violations").and_then(|x| x.as_array()) {
                let mut rules: Vec<String> = arr
                    .iter()
                    .filter_map(|x| {
                        x.get("rule")
                            .and_then(|r| r.get("name"))
                            .and_then(|n| n.as_str())
                    })
                    .map(|s| s.to_string())
                    .collect();
                rules.sort();
                rules.dedup();
                let n = arr.len() as u64;
                let summary = if n == 0 {
                    "no boundary violations".to_string()
                } else {
                    format!("{} violation(s): {}", n, truncate(&rules.join(", "), 200))
                };
                return Some((n, summary));
            }
            let n = ["error", "warn", "info"]
                .iter()
                .filter_map(|k| summary.get(*k).and_then(|x| x.as_u64()))
                .sum();
            Some((
                n,
                if n == 0 {
                    "no boundary violations".to_string()
                } else {
                    format!("{n} violation(s)")
                },
            ))
        },
        "npm install --save-dev dependency-cruiser",
    )
}

fn lychee_inputs(root: &Path) -> Vec<PathBuf> {
    let mut inputs = Vec::new();
    if root.join("docs").exists() {
        inputs.push(PathBuf::from("docs"));
    }
    if root.join("README.md").exists() {
        inputs.push(PathBuf::from("README.md"));
    }
    if inputs.is_empty() {
        inputs.push(PathBuf::from("."));
    }
    inputs
}

fn run_lychee(root: &Path) -> AdapterOutcome {
    const TOOL: &str = "lychee";
    if crate::tools::detect_tool("lychee").available {
        // Present: run below.
    } else {
        return AdapterOutcome {
            tool: TOOL,
            state: "missing".to_string(),
            detail: "lychee absent — install: cargo install lychee (evidence unavailable)"
                .to_string(),
        };
    }
    let inputs = lychee_inputs(root);
    let spawned = Command::new("lychee")
        .arg("--offline")
        .arg("--format")
        .arg("json")
        .arg("--no-progress")
        .args(&inputs)
        .current_dir(root)
        .output();
    classify(
        TOOL,
        spawned,
        |v| {
            let errors = v.get("errors").and_then(|x| x.as_u64())?;
            // Fail maps were renamed fail_map -> error_map across versions.
            let mut bad: Vec<String> = Vec::new();
            for key in ["error_map", "fail_map"] {
                if let Some(map) = v.get(key).and_then(|x| x.as_object()) {
                    bad.extend(map.keys().cloned());
                }
            }
            bad.sort();
            bad.dedup();
            let summary = if errors == 0 {
                "no broken local links or anchors".to_string()
            } else {
                let shown: Vec<String> = bad.iter().take(5).cloned().collect();
                format!(
                    "{} broken link(s): {}",
                    errors,
                    truncate(&shown.join(", "), 200)
                )
            };
            Some((errors, summary))
        },
        "cargo install lychee",
    )
}

fn run_zizmor(root: &Path) -> AdapterOutcome {
    const TOOL: &str = "zizmor";
    if crate::tools::detect_tool("zizmor").available {
        // Present: run below.
    } else {
        return AdapterOutcome {
            tool: TOOL,
            state: "missing".to_string(),
            detail: "zizmor absent — install: cargo install zizmor (evidence unavailable)"
                .to_string(),
        };
    }
    let spawned = Command::new("zizmor")
        .args(["--offline", "--format=json", ".github/workflows"])
        .current_dir(root)
        .output();
    classify(
        TOOL,
        spawned,
        |v| {
            let arr = v.as_array()?;
            let mut kinds: Vec<String> = arr
                .iter()
                .map(|f| {
                    let ident = f.get("ident").and_then(|x| x.as_str()).unwrap_or("finding");
                    let sev = f
                        .pointer("/determinations/severity")
                        .and_then(|x| x.as_str())
                        .unwrap_or("?");
                    format!("{ident} ({sev})")
                })
                .collect();
            kinds.sort();
            kinds.dedup();
            let n = arr.len() as u64;
            let summary = if n == 0 {
                "no workflow findings".to_string()
            } else {
                format!("{} finding(s): {}", n, truncate(&kinds.join(", "), 200))
            };
            Some((n, summary))
        },
        "cargo install zizmor",
    )
}
