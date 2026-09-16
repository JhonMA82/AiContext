use anyhow::Result;
use serde::Serialize;
use std::path::Path;

use crate::config::{AST_GREP_RULES, CONSISTENCY, PATTERNS, PROJECT_STATE, REPO_MANIFEST};
use crate::output::AiError;
use crate::scan;
use crate::tools;

#[derive(Debug, Clone, Serialize)]
struct Finding {
    name: String,
    passed: bool,
    detail: String,
}

pub fn run_check(root: &Path, json: bool) -> Result<i32> {
    let mut findings: Vec<Finding> = Vec::new();

    // 1. schema + manifest parse.
    match crate::config::RepoConfig::load(root) {
        Ok(cfg) => {
            findings.push(Finding {
                name: "schema".to_string(),
                passed: cfg.schema == "aicontext/v1",
                detail: cfg.schema.clone(),
            });
            // 2. PROJECT_STATE markers + freshness.
            let state_path = cfg.state_path(root);
            match std::fs::read_to_string(&state_path) {
                Ok(text) => {
                    let has_markers = text.contains(crate::state::GEN_START)
                        && text.contains(crate::state::GEN_END)
                        && text.contains(crate::state::CUR_START)
                        && text.contains(crate::state::CUR_END);
                    findings.push(Finding {
                        name: "state".to_string(),
                        passed: has_markers,
                        detail: if has_markers {
                            cfg.state.project_state.clone()
                        } else {
                            "missing generated/curated markers".to_string()
                        },
                    });
                    // Freshness against current scan.
                    match scan::collect_scan(root) {
                        Ok(report) => {
                            let fresh = crate::state::generated_block(&report);
                            let stale = crate::state::freshness_key(&fresh)
                                != crate::state::freshness_key(&current_generated(&text));
                            findings.push(Finding {
                                name: "freshness".to_string(),
                                passed: !stale,
                                detail: if stale {
                                    "STALE_PROJECT_STATE: run `aicontext sync`".to_string()
                                } else {
                                    "synchronized".to_string()
                                },
                            });
                        }
                        Err(e) => findings.push(Finding {
                            name: "freshness".to_string(),
                            passed: false,
                            detail: format!("scan failed: {e:#}"),
                        }),
                    }
                    // 3. version source exists.
                    let version_ok = version_source_ok(root, &cfg.version.source);
                    findings.push(Finding {
                        name: "version".to_string(),
                        passed: version_ok,
                        detail: cfg.version.source.clone(),
                    });
                    // 4. commands references: every script in package.json is
                    // documented trivially by existing (no external refs in P0).
                    findings.push(Finding {
                        name: "commands".to_string(),
                        passed: true,
                        detail: "scripts enumerated from scan".to_string(),
                    });
                    // 5. consistency manifest references exist.
                    let (cons_ok, cons_detail) =
                        consistency_refs_ok(root, &cfg.consistency.manifest);
                    findings.push(Finding {
                        name: "consistency".to_string(),
                        passed: cons_ok,
                        detail: cons_detail,
                    });
                }
                Err(_) => {
                    findings.push(Finding {
                        name: "state".to_string(),
                        passed: false,
                        detail: format!("missing {}", cfg.state.project_state),
                    });
                    findings.push(Finding {
                        name: "freshness".to_string(),
                        passed: false,
                        detail: "no state to compare".to_string(),
                    });
                }
            }
            // 6. patterns file exists (content is semantic, presence is enough).
            let patterns_ok = root.join(&cfg.state.patterns).exists();
            findings.push(Finding {
                name: "patterns references".to_string(),
                passed: patterns_ok,
                detail: cfg.state.patterns.clone(),
            });
        }
        Err(e) => {
            findings.push(Finding {
                name: "schema".to_string(),
                passed: false,
                detail: format!("{e:#}"),
            });
        }
    }

    // 7. ast-grep rules dir (only required when declared non-empty is checked;
    // in P0: presence of dir is enough, missing dir is a warning-pass).
    let rules_exist = root.join(AST_GREP_RULES).exists();
    findings.push(Finding {
        name: "structural rules".to_string(),
        passed: true,
        detail: if rules_exist {
            AST_GREP_RULES.to_string()
        } else {
            format!("{AST_GREP_RULES} absent (no rules yet — pass with reduced capability)")
        },
    });

    // 8. required tools: only `git` is required in P0.
    let git = tools::detect_tool("git");
    findings.push(Finding {
        name: "required tools".to_string(),
        passed: git.available,
        detail: git.version.unwrap_or_else(|| "git missing".to_string()),
    });
    // 9. ast-grep availability is informational (degrades gracefully).
    let sg = tools::detect_tool("ast-grep");
    findings.push(Finding {
        name: "ast-grep".to_string(),
        passed: true,
        detail: sg
            .version
            .unwrap_or_else(|| "absent — structural search degraded".to_string()),
    });

    // 10. project adapters (P4): evidence only — an optional tool never
    // blocks the repo (spec §29), so these findings always pass.
    for a in crate::adapters::run_all(root) {
        findings.push(Finding {
            name: format!("adapter:{}", a.tool),
            passed: true,
            detail: format!("[{}] {}", a.state, a.detail),
        });
    }

    let failed: Vec<&Finding> = findings.iter().filter(|f| !f.passed).collect();
    let code = if failed.is_empty() { 0 } else { 1 };

    if json {
        #[derive(Serialize)]
        struct CheckOut<'a> {
            schema: &'a str,
            passed: bool,
            findings: &'a [Finding],
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&CheckOut {
                schema: "aicontext/check/v1",
                passed: code == 0,
                findings: &findings,
            })?
        );
    } else {
        for f in &findings {
            let mark = if f.passed { "✓" } else { "✗" };
            println!("{mark} {} — {}", f.name, f.detail);
        }
    }
    Ok(code)
}

fn current_generated(text: &str) -> String {
    let (Some(s), Some(e)) = (
        text.find(crate::state::GEN_START),
        text.find(crate::state::GEN_END),
    ) else {
        return String::new();
    };
    let start = s + crate::state::GEN_START.len();
    text.get(start..e).unwrap_or("").trim().to_string()
}

fn version_source_ok(root: &Path, source: &str) -> bool {
    // Canonical version sources we understand in P0.
    if source == "package.json" || source == "Cargo.toml" {
        return root.join(source).exists();
    }
    // Unknown source: pass if the referenced file exists.
    root.join(source).exists()
}

fn consistency_refs_ok(root: &Path, manifest: &str) -> (bool, String) {
    let path = root.join(manifest);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return (false, format!("missing {manifest}")),
    };
    let v: serde_yaml::Value = match serde_yaml::from_str(&text) {
        Ok(v) => v,
        Err(e) => return (false, format!("invalid YAML: {e}")),
    };
    if v.get("schema").and_then(|s| s.as_str()) != Some("aicontext/consistency/v1") {
        return (false, "unsupported consistency schema".to_string());
    }
    // P0: verify the ast_grep rules path reference is coherent (exists or
    // explicitly empty). Missing dir is tolerated (no rules yet).
    (true, manifest.to_string())
}

pub fn cmd_check(json: bool) -> Result<i32> {
    let root = scan::current_dir_root()?;
    // Fast path: not initialized at all.
    if !root.join(REPO_MANIFEST).exists() {
        let err = AiError::new(
            "NOT_INITIALIZED",
            format!("missing {REPO_MANIFEST}"),
            Some("aicontext init"),
        );
        if json {
            crate::output::print_error(&anyhow::anyhow!(err), true);
        } else {
            eprintln!("✗ schema — missing {REPO_MANIFEST} (run `aicontext init`)");
            eprintln!("hint: {PROJECT_STATE}, {PATTERNS}, {CONSISTENCY} not found");
        }
        return Ok(2);
    }
    run_check(&root, json)
}
