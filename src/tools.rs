use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub available: bool,
    pub version: Option<String>,
    pub managed_by: String,
}

fn probe(program: &str) -> Option<String> {
    let out = Command::new(program).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let first = text.lines().next().unwrap_or("").trim().to_string();
    if first.is_empty() {
        Some("unknown".to_string())
    } else {
        Some(first)
    }
}

pub fn detect_tool(name: &str) -> ToolInfo {
    let program: &str = name;
    let version = probe(program).or_else(|| {
        if name == "ast-grep" {
            probe("sg")
        } else {
            None
        }
    });
    ToolInfo {
        name: name.to_string(),
        available: version.is_some(),
        version,
        managed_by: "external".to_string(),
    }
}

pub fn detect_core_tools() -> Vec<ToolInfo> {
    ["git", "ast-grep", "rg", "tgrep", "rtk", "codegraph"]
        .iter()
        .map(|n| detect_tool(n))
        .collect()
}

/// A single recommendation row in `tools plan`.
#[derive(Debug, Clone, Serialize)]
pub struct PlanEntry {
    pub tool: String,
    pub level: String,
    pub state: String,
    pub reason: String,
    pub version: Option<String>,
    pub install: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanSection {
    pub title: String,
    pub entries: Vec<PlanEntry>,
}

fn entry(
    tool: &str,
    level: &str,
    reason: &str,
    install: Option<&str>,
    disabled: bool,
) -> PlanEntry {
    if disabled {
        return PlanEntry {
            tool: tool.to_string(),
            level: level.to_string(),
            state: "disabled".to_string(),
            reason: "mode = off in aicontext.toml".to_string(),
            version: None,
            install: None,
        };
    }
    let info = detect_tool(tool);
    PlanEntry {
        tool: tool.to_string(),
        level: level.to_string(),
        state: if info.available {
            "available".to_string()
        } else {
            "missing".to_string()
        },
        reason: reason.to_string(),
        version: info.version,
        install: install.map(|s| s.to_string()),
    }
}

fn is_js_ts(root: &std::path::Path) -> bool {
    root.join("package.json").exists()
}

fn has_github_actions(root: &std::path::Path) -> bool {
    root.join(".github/workflows").exists()
}

fn is_docs_heavy(root: &std::path::Path, report: &crate::scan::ScanReport) -> bool {
    root.join("docs").exists()
        || report
            .languages
            .iter()
            .any(|l| l.language == "markdown" && l.files >= 10)
}

/// Build the plan. Read-only: never installs anything.
/// `mode()` resolves the per-tool mode from aicontext.toml (default auto).
pub(crate) fn build_plan(root: &std::path::Path) -> Vec<PlanSection> {
    let report = crate::scan::collect_scan(root).unwrap_or_else(|_| empty_report(root));
    let cfg = crate::config::RepoConfig::load(root).ok();
    let mode = |name: &str, fallback: &str| -> String {
        match (cfg.as_ref(), name) {
            (Some(c), "tgrep") => c.tools.tgrep.mode.clone(),
            (Some(c), "codegraph") => c.tools.codegraph.mode.clone(),
            (Some(c), "rtk") => c.tools.rtk.mode.clone(),
            _ => fallback.to_string(),
        }
    };
    let off = |name: &str| mode(name, "auto") == "off";
    let profile = report.complexity.profile.as_str();
    let medium_or_large = profile == "medium" || profile == "large";
    let large = profile == "large";

    let required = vec![entry(
        "git",
        "required",
        "Git is the source of truth for root/HEAD/tracked files",
        None,
        false,
    )];
    let core = vec![entry(
        "ast-grep",
        "core",
        "structural search, lint rules and explicit rewrites",
        Some("npm i -g @ast-grep/cli | cargo install ast-grep --locked | brew install ast-grep"),
        false,
    )];
    let mut recommended = Vec::new();
    recommended.push(entry(
"tgrep",
"recommended",
if medium_or_large {
"medium/large repository: indexed text search"
} else {
"small repository: optional, rg/git-grep suffice"
},
Some("brew install tgrep | cargo install --path tgrep-cli --locked (managed install verifies checksums)"),
off("tgrep"),
));
    recommended.push(entry(
        "rtk",
        "recommended",
        "shell output compression for Pi sessions (verify with `rtk gain`)",
        Some("install Rust Token Killer from rtk-ai/rtk, then `rtk init -g --agent pi`"),
        off("rtk"),
    ));
    if large || report.complexity.workspaces >= 4 {
        recommended.push(entry(
            "codegraph",
            "recommended",
            "large multi-package repo: callers/callees and blast radius without broad reads",
            Some("download the official engine binary + checksum for your platform"),
            off("codegraph"),
        ));
    }
    let mut adapters = Vec::new();
    if is_js_ts(root) {
        adapters.push(entry(
            "knip",
            "adapter",
            "JS/TS detected: unused files, dependencies and exports (project-local, evidence only)",
            Some("npm install --save-dev knip (never global by default)"),
            false,
        ));
    }
    if root.join("docs").exists() || is_docs_heavy(root, &report) {
        adapters.push(entry(
            "lychee",
            "adapter",
            "docs-heavy repo: broken relative links and anchors (offline checks first)",
            Some("cargo install lychee"),
            false,
        ));
    }
    if has_github_actions(root) {
        adapters.push(entry(
            "zizmor",
            "adapter",
            ".github/workflows detected: static/security analysis of Actions",
            Some("cargo install zizmor"),
            false,
        ));
    }
    vec![
        PlanSection {
            title: "Required".to_string(),
            entries: required,
        },
        PlanSection {
            title: "Core".to_string(),
            entries: core,
        },
        PlanSection {
            title: "Recommended".to_string(),
            entries: recommended,
        },
        PlanSection {
            title: "Project adapters".to_string(),
            entries: adapters,
        },
    ]
}

fn empty_report(root: &std::path::Path) -> crate::scan::ScanReport {
    crate::scan::ScanReport {
        schema: "aicontext/scan/v1".to_string(),
        git: crate::scan::GitInfo {
            root: root.to_string_lossy().to_string(),
            head: None,
            short_head: None,
            branch: None,
            tracked_files: 0,
        },
        languages: Vec::new(),
        packages: Vec::new(),
        commands: Vec::new(),
        tools: std::collections::HashMap::new(),
        complexity: crate::scan::ComplexityInfo {
            profile: "small".to_string(),
            source_files: 0,
            loc: 0,
            workspaces: 0,
            languages: 0,
            reason: "scan unavailable".to_string(),
        },
        version_candidates: Vec::new(),
        docs: Vec::new(),
        ci: Vec::new(),
    }
}

pub fn cmd_plan(json: bool) -> anyhow::Result<i32> {
    let root = crate::scan::current_dir_root()?;
    let sections = build_plan(&root);
    if json {
        #[derive(Serialize)]
        struct PlanOut<'a> {
            schema: &'a str,
            sections: &'a [PlanSection],
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&PlanOut {
                schema: "aicontext/tools-plan/v1",
                sections: &sections,
            })?
        );
        return Ok(0);
    }
    for section in &sections {
        println!("{}:", section.title);
        if section.entries.is_empty() {
            println!("  (none applicable)");
        }
        for e in &section.entries {
            let mark = match e.state.as_str() {
                "available" => "✓",
                "disabled" => "-",
                _ => "!",
            };
            let ver = e.version.as_deref().unwrap_or("");
            println!("  {mark} {} {ver}", e.tool);
            println!("      {}: {}", e.level, e.reason);
            if e.state == "missing" {
                if let Some(hint) = &e.install {
                    println!("      install: {hint}");
                }
            }
        }
        println!();
    }
    println!("`tools plan` never installs. Confirm explicitly before any install.");
    Ok(0)
}

pub fn cmd_status(json: bool) -> anyhow::Result<i32> {
    let root = crate::scan::current_dir_root()?;
    let _ = root;
    let mut tools = detect_core_tools();
    // Overlay registry ownership: managed tools report managed_by=managed
    // with the verified installed version, even off PATH.
    if let Ok(prefix) = crate::tools_install::managed_prefix() {
        let registry = crate::tools_install::load_registry(&prefix);
        for t in tools.iter_mut() {
            if let Some(owned) = registry.tools.iter().find(|m| m.name == t.name) {
                t.managed_by = "managed".to_string();
                t.version = Some(owned.version.clone());
                t.available = true;
            }
        }
    }
    if json {
        #[derive(Serialize)]
        struct StatusOut<'a> {
            schema: &'a str,
            tools: &'a [ToolInfo],
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&StatusOut {
                schema: "aicontext/tools-status/v1",
                tools: &tools,
            })?
        );
        return Ok(0);
    }
    for t in &tools {
        match (&t.available, &t.version) {
            (true, Some(v)) => println!("✓ {} — {} (managed_by={})", t.name, v, t.managed_by),
            (true, None) => println!("✓ {} (managed_by={})", t.name, t.managed_by),
            (false, _) => println!("! {} — not installed (managed_by={})", t.name, t.managed_by),
        }
    }
    if let Some(warn) = rtk_health() {
        println!("warning: {warn}");
    }
    Ok(0)
}

/// Health check for the RTK name-collision trap: a different package named
/// `rtk` exists, so `rtk gain` must succeed for Rust Token Killer.
pub fn rtk_health() -> Option<String> {
    let out = Command::new("rtk").arg("gain").output().ok()?;
    if out.status.success() {
        None
    } else {
        Some("RTK binary exists but `rtk gain` fails: likely a different package named rtk is installed. Install Rust Token Killer from rtk-ai/rtk.".to_string())
    }
}
