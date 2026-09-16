use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

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
    // Facts captured from the single scan below; reused by the commands and
    // projections checks so `check` compares declarations against reality.
    let mut scan_commands: Vec<String> = Vec::new();
    let mut scan_versions: Vec<(String, String)> = Vec::new();
    let mut scan_ok = false;
    // Manifest path for the adapter policy table (section 10).
    let mut consistency_manifest = CONSISTENCY.to_string();

    // 1. schema + manifest parse.
    match crate::config::RepoConfig::load(root) {
        Ok(cfg) => {
            consistency_manifest = cfg.consistency.manifest.clone();
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
                            scan_commands = report.commands.clone();
                            scan_versions = report
                                .version_candidates
                                .iter()
                                .map(|v| (v.source.clone(), v.version.clone()))
                                .collect();
                            scan_ok = true;
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
                    // 4. commands: detected scripts must match the declared
                    // list exactly — missing entries and stale ones both fail.
                    // 5. consistency: projections and protected paths validated.
                    match crate::config::ConsistencyFile::load(root, &cfg.consistency.manifest) {
                        Ok(cons) => {
                            let (cmd_ok, cmd_detail) = if scan_ok {
                                check_commands(root, &scan_commands, &cons.commands.documented)
                            } else {
                                (false, "scan failed: cannot verify commands".to_string())
                            };
                            findings.push(Finding {
                                name: "commands".to_string(),
                                passed: cmd_ok,
                                detail: cmd_detail,
                            });
                            let (cons_ok, cons_detail) = check_consistency(
                                root,
                                &cfg.consistency.manifest,
                                &cons,
                                &cfg.version.source,
                                &scan_versions,
                                scan_ok,
                            );
                            findings.push(Finding {
                                name: "consistency".to_string(),
                                passed: cons_ok,
                                detail: cons_detail,
                            });
                        }
                        Err(e) => {
                            let detail = format!("{:#}", e);
                            findings.push(Finding {
                                name: "commands".to_string(),
                                passed: false,
                                detail: format!("cannot verify: {detail}"),
                            });
                            findings.push(Finding {
                                name: "consistency".to_string(),
                                passed: false,
                                detail,
                            });
                        }
                    }
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

    // 7. structural rules: every declared ast-grep rule file is executed;
    // a match is a violation. No rule files means nothing to enforce.
    // Rules present but ast-grep absent fail closed — silently skipping
    // declared rules is exactly the gap this gate closes.
    let (rules_ok, rules_detail) = check_structural_rules(root);
    findings.push(Finding {
        name: "structural rules".to_string(),
        passed: rules_ok,
        detail: rules_detail,
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

    // 10. project adapters (P4): advisory by default (evidence only — an
    // optional tool never blocks the repo without explicit configuration,
    // spec §29). A `required` policy in consistency.yml turns that adapter
    // into a gate: only a clean executed run passes.
    for f in check_adapters(root, &consistency_manifest) {
        findings.push(f);
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

/// Policy-aware adapter findings. The `aicontext/check/v1` shape is
/// unchanged (schema/passed/findings with name/passed/detail); only
/// pass/fail and detail strings reflect policy.
fn check_adapters(root: &Path, manifest: &str) -> Vec<Finding> {
    let policies = adapter_policies(root, manifest);
    let outcomes = crate::adapters::run_all(root);
    let mut findings = Vec::new();
    // The table itself fails closed: unknown names (typos) and invalid
    // values block the repo with one finding naming every problem.
    let mut config_errors: Vec<String> = Vec::new();
    for (name, policy) in &policies {
        if crate::adapters::KNOWN_ADAPTERS.contains(&name.as_str()) {
            if policy == "required" || policy == "advisory" {
                continue;
            }
            config_errors.push(format!(
                "invalid policy {policy:?} for adapter {name:?} — expected \"required\" or \"advisory\""
            ));
        } else {
            config_errors.push(format!(
                "unknown adapter {name:?} — known adapters: {}",
                crate::adapters::KNOWN_ADAPTERS.join(", ")
            ));
        }
    }
    if config_errors.is_empty() {
    } else {
        findings.push(Finding {
            name: "adapter policy".to_string(),
            passed: false,
            detail: format!("{}: {}", manifest, config_errors.join("; ")),
        });
    }
    for a in &outcomes {
        let policy = policies
            .get(a.tool)
            .map(|s| s.as_str())
            .unwrap_or("advisory");
        if policy == "required" {
            let (passed, detail) = required_adapter_verdict(a, manifest);
            findings.push(Finding {
                name: format!("adapter:{}", a.tool),
                passed,
                detail,
            });
        } else {
            // Advisory (the default): evidence only, always passes — even
            // when the declared value is invalid, since that misconfig
            // already fails through the `adapter policy` finding above.
            findings.push(Finding {
                name: format!("adapter:{}", a.tool),
                passed: true,
                detail: format!("[{}] {}", a.state, a.detail),
            });
        }
    }
    // A required adapter that never ran (not applicable to this repo)
    // fails closed: silently passing would pretend the gate is enforced.
    for (name, policy) in &policies {
        if policy == "required" {
            match outcomes.iter().any(|a| a.tool == name.as_str()) {
                true => {}
                false => {
                    if crate::adapters::KNOWN_ADAPTERS.contains(&name.as_str()) {
                        findings.push(Finding {
                            name: format!("adapter:{name}"),
                            passed: false,
                            detail: format!(
                                "{name} is required by {manifest} but did not run (not applicable to this repo) — remove the policy, make the repo applicable, or set policy to advisory"
                            ),
                        });
                    }
                }
            }
        }
    }
    findings
}

/// Loads the per-adapter policy table (`adapters.<tool>.policy`). A
/// missing or unreadable manifest means no policies (everything advisory);
/// the commands/consistency findings already report that load failure.
fn adapter_policies(root: &Path, manifest: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(cons) = crate::config::ConsistencyFile::load(root, manifest) {
        for (name, entry) in cons.adapters {
            out.insert(name, entry.policy);
        }
    }
    out
}

/// A required adapter passes only on a clean executed run. Anything else
/// fails with the exact remediation: fix the tool state, or relax the
/// policy back to advisory.
fn required_adapter_verdict(a: &crate::adapters::AdapterOutcome, manifest: &str) -> (bool, String) {
    if a.state == "ok" {
        return (
            true,
            format!("[ok] {} (policy required by {manifest}: clean)", a.detail),
        );
    }
    let action = match a.state.as_str() {
        "issues" => "fix the reported issues",
        "missing" => "install the adapter so evidence is available",
        "unconfigured" => "add the expected config so the adapter can run",
        _ => "fix the adapter run failure",
    };
    (
        false,
        format!(
            "[{}] {} — required by {manifest} (adapters.{}.policy = required): {action}, or set policy to advisory",
            a.state, a.detail, a.tool
        ),
    )
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

/// Two-way reconciliation of detected scripts against the declared list.
/// Undocumented scripts fail (new behavior must be declared) and stale
/// entries fail (removed behavior must stop being referenced). For stale
/// names, docs still mentioning them are listed as exact remediation sites.
fn check_commands(root: &Path, actual: &[String], documented: &[String]) -> (bool, String) {
    let actual_set: BTreeSet<&str> = actual.iter().map(|s| s.as_str()).collect();
    let documented_set: BTreeSet<&str> = documented.iter().map(|s| s.as_str()).collect();
    let undocumented: Vec<&str> = actual_set.difference(&documented_set).copied().collect();
    let stale: Vec<&str> = documented_set.difference(&actual_set).copied().collect();
    if undocumented.is_empty() && stale.is_empty() {
        return (
            true,
            if actual.is_empty() {
                "no scripts detected, none documented".to_string()
            } else {
                let mut names: Vec<&str> = actual_set.into_iter().collect();
                names.sort();
                format!("{} command(s) verified: {}", names.len(), names.join(", "))
            },
        );
    }
    let mut parts: Vec<String> = Vec::new();
    if !undocumented.is_empty() {
        parts.push(format!(
            "undocumented: {} — declare in consistency.yml",
            undocumented.join(", ")
        ));
    }
    if !stale.is_empty() {
        let stale_owned: Vec<String> = stale.iter().map(|s| s.to_string()).collect();
        let mut msg = format!("stale: {} — remove from consistency.yml", stale.join(", "));
        let mentions = stale_mentions(root, &stale_owned);
        if !mentions.is_empty() {
            msg.push_str(&format!("; still mentioned in: {}", mentions.join(", ")));
        }
        parts.push(msg);
    }
    (false, parts.join("; "))
}

/// Markdown docs shipped with the repo; bounded so a docs-heavy tree can
/// never make `check` crawl the world.
fn doc_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for f in ["README.md", "AGENTS.md", "CHANGELOG.md"] {
        let p = root.join(f);
        if p.is_file() {
            out.push(p);
        }
    }
    let mut stack = vec![root.join("docs")];
    while let Some(dir) = stack.pop() {
        if out.len() >= 200 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                out.push(p);
            }
        }
    }
    out
}

/// Root-relative files whose text literally mentions one of `names`.
fn stale_mentions(root: &Path, names: &[String]) -> Vec<String> {
    let mut hits: Vec<String> = Vec::new();
    for path in doc_files(root) {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() > 1_000_000 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if names.iter().any(|n| text.contains(n)) {
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string_lossy().to_string());
            hits.push(rel);
            if hits.len() >= 5 {
                break;
            }
        }
    }
    hits.sort();
    hits
}

/// Validates the consistency manifest beyond its schema: every version
/// projection must contain the resolved version, and every protected path
/// must exist. An empty list enforces nothing and passes.
fn check_consistency(
    root: &Path,
    manifest: &str,
    cons: &crate::config::ConsistencyFile,
    version_source: &str,
    scan_versions: &[(String, String)],
    scan_ok: bool,
) -> (bool, String) {
    let mut ok = true;
    let mut parts: Vec<String> = Vec::new();

    if cons.version.projections.is_empty() {
        parts.push("no projections declared".to_string());
    } else if scan_ok {
        match scan_versions
            .iter()
            .find(|(source, _)| source == version_source)
        {
            None => {
                ok = false;
                parts.push(format!("cannot resolve version from {version_source}"));
            }
            Some((_, version)) => {
                let mut bad: Vec<String> = Vec::new();
                for proj in &cons.version.projections {
                    match projection_ok(root, proj, version) {
                        Ok(()) => {}
                        Err(reason) => bad.push(format!("{proj} ({reason})")),
                    }
                }
                if bad.is_empty() {
                    parts.push(format!(
                        "{} projection(s) carry {version}",
                        cons.version.projections.len()
                    ));
                } else {
                    ok = false;
                    parts.push(format!("projections failing: {}", bad.join(", ")));
                }
            }
        }
    } else {
        ok = false;
        parts.push("scan failed: cannot verify projections".to_string());
    }

    if cons.protected.is_empty() {
        parts.push("no protected paths declared".to_string());
    } else {
        let missing: Vec<&str> = cons
            .protected
            .iter()
            .filter(|p| !root.join(p).exists())
            .map(|p| p.as_str())
            .collect();
        if missing.is_empty() {
            parts.push(format!(
                "{} protected path(s) present",
                cons.protected.len()
            ));
        } else {
            ok = false;
            parts.push(format!("protected missing: {}", missing.join(", ")));
        }
    }

    (ok, format!("{}: {}", manifest, parts.join("; ")))
}

fn projection_ok(root: &Path, proj: &str, version: &str) -> Result<(), String> {
    let rel = Path::new(proj);
    if rel.is_absolute() || proj.contains("..") {
        return Err("unsafe path".to_string());
    }
    let path = root.join(rel);
    let meta = std::fs::metadata(&path).map_err(|_| "missing file".to_string())?;
    if !meta.is_file() {
        return Err("not a file".to_string());
    }
    if meta.len() > 1_000_000 {
        return Err("file too large to verify".to_string());
    }
    let text = std::fs::read_to_string(&path).map_err(|_| "unreadable".to_string())?;
    if text.contains(version) {
        Ok(())
    } else {
        Err(format!("does not contain {version}"))
    }
}

/// Executes every ast-grep rule file under the declared rules directory.
/// Any match is a violation and fails the gate.
fn check_structural_rules(root: &Path) -> (bool, String) {
    let configured = crate::config::ConsistencyFile::load(root, &default_consistency_manifest())
        .map(|c| c.checks.ast_grep.rules)
        .unwrap_or_else(|_| AST_GREP_RULES.to_string());
    let dir = root.join(&configured);
    let rules = collect_rule_files(&dir);
    if rules.is_empty() {
        return (
            true,
            format!("no rules in {configured} — nothing to enforce"),
        );
    }
    match crate::tools::detect_tool("ast-grep").available {
        true => {}
        false => {
            return (
                false,
                format!(
                    "{} rule(s) declared but ast-grep is absent — run `aicontext tools install ast-grep`",
                    rules.len()
                ),
            );
        }
    }
    let mut clean = 0usize;
    let mut problems: Vec<String> = Vec::new();
    for rule in &rules {
        let name = rule
            .strip_prefix(&dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| rule.to_string_lossy().to_string());
        match run_rule(root, rule) {
            RuleOutcome::Clean => clean += 1,
            RuleOutcome::Violations(summary) => {
                problems.push(format!("{name}: {summary}"));
            }
            RuleOutcome::Error(reason) => {
                problems.push(format!("{name}: {reason}"));
            }
        }
    }
    if problems.is_empty() {
        (true, format!("{} rule(s) executed, no violations", clean))
    } else {
        (
            false,
            format!("{} violation(s): {}", problems.len(), problems.join("; ")),
        )
    }
}

fn default_consistency_manifest() -> String {
    CONSISTENCY.to_string()
}

enum RuleOutcome {
    Clean,
    Violations(String),
    Error(String),
}

fn collect_rule_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        if out.len() >= 100 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            // Never lint our own managed state as target noise; rule files
            // themselves live here, so only collect rule definitions.
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "yml" || x == "yaml") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn run_rule(root: &Path, rule: &Path) -> RuleOutcome {
    let mut cmd = Command::new("ast-grep");
    cmd.arg("scan")
        .arg("--rule")
        .arg(rule)
        .arg("--json")
        .arg(".")
        .current_dir(root);
    let Some(out) = crate::output::command_output(cmd, 60) else {
        return RuleOutcome::Error("could not execute (timeout or spawn failed)".to_string());
    };
    match sg_matches(&out.stdout) {
        Some(matches) if !matches.is_empty() => {
            let errors = matches.iter().filter(|(_, s)| s == "error").count();
            let warnings = matches.len() - errors;
            let mut files: Vec<&str> = matches.iter().map(|(f, _)| f.as_str()).collect();
            files.sort();
            files.dedup();
            let shown: Vec<&str> = files.into_iter().take(3).collect();
            let mut summary = String::new();
            if errors > 0 {
                summary.push_str(&format!("{errors} error(s)"));
            }
            if warnings > 0 {
                if !summary.is_empty() {
                    summary.push_str(", ");
                }
                summary.push_str(&format!("{warnings} warning(s)"));
            }
            summary.push_str(&format!(" in {}", shown.join(", ")));
            RuleOutcome::Violations(summary)
        }
        Some(_) => RuleOutcome::Clean,
        None => {
            if out.status.success() {
                RuleOutcome::Clean
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let tail: String = stderr
                    .trim()
                    .chars()
                    .rev()
                    .take(200)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                RuleOutcome::Error(format!(
                    "rule failed to run{}",
                    if tail.is_empty() {
                        String::new()
                    } else {
                        format!(": {tail}")
                    }
                ))
            }
        }
    }
}

/// Extracts `(file, severity)` pairs from `ast-grep scan --json` stdout.
/// Falls back to locating the JSON array when the binary wraps it.
fn sg_matches(stdout: &[u8]) -> Option<Vec<(String, String)>> {
    if stdout.iter().all(|b| b.is_ascii_whitespace()) {
        return Some(Vec::new());
    }
    if let Some(parsed) = parse_match_array(stdout) {
        return Some(parsed);
    }
    let start = stdout.iter().position(|&b| b == b'[')?;
    let end = stdout.iter().rposition(|&b| b == b']')?;
    if end <= start {
        return None;
    }
    parse_match_array(stdout.get(start..=end)?)
}

fn parse_match_array(bytes: &[u8]) -> Option<Vec<(String, String)>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let arr = value.as_array()?;
    let mut out = Vec::new();
    for entry in arr {
        let file = entry
            .get("file")
            .and_then(|f| f.as_str())
            .unwrap_or("?")
            .to_string();
        let severity = entry
            .get("severity")
            .and_then(|s| s.as_str())
            .unwrap_or("error")
            .to_string();
        out.push((file, severity));
    }
    Some(out)
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
