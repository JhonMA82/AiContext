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

/// Health check for the RTK name-collision trap: a different package named
/// `rtk` exists, so `rtk gain` must succeed for Rust Token Killer.
#[allow(dead_code)]
pub fn rtk_health() -> Option<String> {
    let out = Command::new("rtk").arg("gain").output().ok()?;
    if out.status.success() {
        None
    } else {
        Some("RTK binary exists but `rtk gain` fails: likely a different package named rtk is installed. Install Rust Token Killer from rtk-ai/rtk.".to_string())
    }
}
