use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::config::{
    self, RepoConfig, AST_GREP_RULES, CONSISTENCY, PATTERNS, PROJECT_STATE, REPO_MANIFEST,
};
use crate::output::AiError;
use crate::scan::{self, ScanReport};

pub const GEN_START: &str = "<!-- aicontext:generated:start -->";
pub const GEN_END: &str = "<!-- aicontext:generated:end -->";
pub const CUR_START: &str = "<!-- aicontext:curated:start -->";
pub const CUR_END: &str = "<!-- aicontext:curated:end -->";

fn repo_name_from_root(root: &Path) -> String {
    root.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn ensure_gitignore(root: &Path) -> Result<bool> {
    let path = root.join(".gitignore");
    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(&path)?
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        Vec::new()
    };
    let mut changed = false;
    for entry in [".aicontext/", ".tgrep/"] {
        if !lines.iter().any(|l| l.trim() == entry) {
            lines.push(entry.to_string());
            changed = true;
        }
    }
    if changed {
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(&path, text)?;
    }
    Ok(changed)
}

pub(crate) fn generated_block(report: &ScanReport) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Last synchronized commit: {}\n",
        report.git.short_head.as_deref().unwrap_or("unknown")
    ));
    if let Some(v) = report.version_candidates.first() {
        s.push_str(&format!("Version: {} ({})\n", v.version, v.source));
    }
    for p in &report.packages {
        s.push_str(&format!(
            "Package manager: {} ({})\n",
            p.manager, p.manifest
        ));
        if !p.workspaces.is_empty() {
            s.push_str(&format!("Workspaces: {}\n", p.workspaces.len()));
        }
    }
    s.push_str(&format!("Complexity: {}\n", report.complexity.profile));
    s.push_str(&format!(
        "Source files: {} | LOC: ~{}\n",
        report.complexity.source_files, report.complexity.loc
    ));
    s.push_str("\nImportant paths:\n");
    for d in &report.docs {
        s.push_str(&format!("- {d}\n"));
    }
    if !report.commands.is_empty() {
        s.push_str("\nCommands:\n");
        let mut cmds = report.commands.clone();
        cmds.sort();
        for c in cmds.iter().take(20) {
            s.push_str(&format!("- {c}\n"));
        }
    }
    s
}

fn render_project_state(generated: &str, curated: Option<&str>) -> String {
    format!(
        "# Project State\n\n{GEN_START}\n{generated}{GEN_END}\n\n{CUR_START}\n{curated}{CUR_END}\n",
        curated = curated.unwrap_or(
            "## Purpose\nTBD — describe what this project is for.\n\n## Current capabilities\n- TBD\n\n## Constraints\n- TBD\n"
        )
    )
}

fn extract_curated(existing: &str) -> Option<String> {
    let start = existing.find(CUR_START)? + CUR_START.len();
    let end = existing.find(CUR_END)?;
    if end < start {
        return None;
    }
    let body = existing.get(start..end)?;
    Some(body.trim().to_string() + "\n")
}

fn extract_generated(existing: &str) -> Option<String> {
    let start = existing.find(GEN_START)? + GEN_START.len();
    let end = existing.find(GEN_END)?;
    if end < start {
        return None;
    }
    Some(existing.get(start..end)?.to_string())
}

pub fn cmd_init(profile: Option<String>, _non_interactive: bool, json: bool) -> Result<i32> {
    let root = scan::current_dir_root()?;
    let profile = profile.unwrap_or_else(|| "auto".to_string());
    let eng = root.join(".engineering");
    std::fs::create_dir_all(&eng)?;
    std::fs::create_dir_all(eng.join("rules/ast-grep"))?;

    // aicontext.toml — never overwrite an existing manifest.
    let manifest_path = root.join(REPO_MANIFEST);
    if !manifest_path.exists() {
        let name = repo_name_from_root(&root);
        std::fs::write(&manifest_path, config::minimal_toml(&name, &profile))?;
    }
    // consistency.yml stub.
    let consistency_path = root.join(CONSISTENCY);
    if !consistency_path.exists() {
        std::fs::write(&consistency_path, config::minimal_consistency())?;
    }
    // PATTERNS.md placeholder (semantic content belongs to the skill).
    let patterns_path = root.join(PATTERNS);
    if !patterns_path.exists() {
        std::fs::write(
            &patterns_path,
            "# Patterns\n\n> Semantic adoption (`aicontext-adopt` skill) fills this file.\n> Status values: preferred | observed | legacy | exception | protected.\n",
        )?;
    }
    // PROJECT_STATE.md — preserve curated, refresh generated.
    let report = scan::collect_scan(&root)?;
    let state_path = root.join(PROJECT_STATE);
    let curated = std::fs::read_to_string(&state_path)
        .ok()
        .as_deref()
        .and_then(extract_curated);
    let generated = generated_block(&report);
    std::fs::write(
        &state_path,
        render_project_state(&generated, curated.as_deref()),
    )?;

    ensure_gitignore(&root)?;

    // Run check to report what is still missing.
    let check_code = crate::check::run_check(&root, json)?;
    if json {
        #[derive(Serialize)]
        struct InitOut<'a> {
            schema: &'a str,
            initialized: bool,
            root: &'a str,
            check_passed: bool,
        }
        let root_str = root.to_string_lossy().into_owned();
        let out = InitOut {
            schema: "aicontext/init/v1",
            initialized: true,
            root: &root_str,
            check_passed: check_code == 0,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("Initialized AIContext in {}", root.to_string_lossy());
        println!("Semantic TODO: fill Purpose/Capabilities/Constraints in {PROJECT_STATE}, then adopt patterns in {PATTERNS}.");
    }
    Ok(check_code)
}

pub fn cmd_sync(check_only: bool, json: bool) -> Result<i32> {
    let root = scan::current_dir_root()?;
    let cfg = RepoConfig::load(&root)
        .map_err(|e| AiError::new("NOT_INITIALIZED", format!("{e:#}"), Some("aicontext init")))?;
    let state_path: PathBuf = cfg.state_path(&root);
    let existing = std::fs::read_to_string(&state_path)
        .with_context(|| format!("missing {}", cfg.state.project_state))?;
    let report = scan::collect_scan(&root)?;
    let fresh_generated = generated_block(&report);
    let current_generated = extract_generated(&existing).unwrap_or_default();
    let drift = current_generated.trim() != fresh_generated.trim();

    if check_only {
        if json {
            #[derive(Serialize)]
            struct SyncCheck<'a> {
                schema: &'a str,
                synchronized: bool,
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&SyncCheck {
                    schema: "aicontext/sync/v1",
                    synchronized: !drift,
                })?
            );
        } else if drift {
            println!(
                "stale: PROJECT_STATE generated block differs from scan; run `aicontext sync`."
            );
        } else {
            println!("synchronized.");
        }
        return Ok(if drift { 1 } else { 0 });
    }

    if drift {
        let curated = extract_curated(&existing);
        std::fs::write(
            &state_path,
            render_project_state(&fresh_generated, curated.as_deref()),
        )?;
    }
    if json {
        #[derive(Serialize)]
        struct SyncOut<'a> {
            schema: &'a str,
            updated: bool,
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&SyncOut {
                schema: "aicontext/sync/v1",
                updated: drift,
            })?
        );
    } else if drift {
        println!("Synced generated block in {}", cfg.state.project_state);
    } else {
        println!("Already synchronized (zero diff).");
    }
    Ok(0)
}

pub fn cmd_status(json: bool) -> Result<i32> {
    let root = scan::current_dir_root()?;
    let cfg = RepoConfig::load(&root);
    let report = scan::collect_scan(&root)?;
    let (state_label, last_check) = match &cfg {
        Ok(c) => {
            let state_path = c.state_path(&root);
            let stale = std::fs::read_to_string(&state_path)
                .ok()
                .as_deref()
                .and_then(extract_generated)
                .map(|g| g.trim() != generated_block(&report).trim())
                .unwrap_or(true);
            (if stale { "stale" } else { "synchronized" }, "unknown")
        }
        Err(_) => ("uninitialized", "unknown"),
    };
    if json {
        #[derive(Serialize)]
        struct StatusOut<'a> {
            schema: &'a str,
            repo: &'a str,
            state: &'a str,
            head: Option<&'a str>,
            complexity: &'a str,
            last_check: &'a str,
        }
        let root_str = root.to_string_lossy().into_owned();
        let out = StatusOut {
            schema: "aicontext/status/v1",
            repo: &root_str,
            state: state_label,
            head: report.git.short_head.as_deref(),
            complexity: &report.complexity.profile,
            last_check,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(0);
    }
    println!("AIContext v0.1");
    println!("Repo: {}", root.to_string_lossy());
    println!("State: {state_label}");
    println!("HEAD: {}", report.git.short_head.as_deref().unwrap_or("?"));
    println!("Complexity: {}", report.complexity.profile);
    println!();
    println!("Search:");
    println!(
        "  text: {}",
        report
            .tools
            .get("tgrep")
            .map(|t| if t.available {
                "tgrep ✓"
            } else {
                "rg/git-grep fallback"
            })
            .unwrap_or("rg/git-grep fallback")
    );
    println!(
        "  structural: {}",
        report
            .tools
            .get("ast-grep")
            .map(|t| if t.available {
                "ast-grep ✓"
            } else {
                "unavailable (reduced capability)"
            })
            .unwrap_or("unavailable")
    );
    println!("  graph: disabled (P3)");
    println!();
    println!("Last check: {last_check}");
    if state_label == "uninitialized" {
        println!("Hint: run `aicontext init`.");
    } else if state_label == "stale" {
        println!("Hint: run `aicontext sync`.");
    }
    let _ = AST_GREP_RULES;
    Ok(0)
}
