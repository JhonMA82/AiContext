use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::RepoConfig;
use crate::output::AiError;
use crate::tools::{self, ToolInfo};

#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    pub schema: String,
    pub git: GitInfo,
    pub languages: Vec<LanguageStat>,
    pub packages: Vec<PackageInfo>,
    pub commands: Vec<String>,
    pub tools: HashMap<String, ToolInfo>,
    pub complexity: ComplexityInfo,
    pub version_candidates: Vec<VersionCandidate>,
    pub docs: Vec<String>,
    pub ci: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitInfo {
    pub root: String,
    pub head: Option<String>,
    pub short_head: Option<String>,
    pub branch: Option<String>,
    pub tracked_files: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageStat {
    pub language: String,
    pub files: usize,
    pub loc: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageInfo {
    pub manager: String,
    pub manifest: String,
    pub workspaces: Vec<String>,
    pub scripts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComplexityInfo {
    pub profile: String,
    pub source_files: usize,
    pub loc: u64,
    pub workspaces: usize,
    pub languages: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionCandidate {
    pub source: String,
    pub version: String,
}

fn git_output<const N: usize>(root: &Path, args: [&str; N]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(args);
    let out = crate::output::command_output(cmd, 15)?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn find_git_root(start: &Path) -> Result<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(start)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("git rev-parse failed; is this a git repository?")?;
    if !out.status.success() {
        anyhow::bail!("not a git repository (git rev-parse failed)");
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

pub fn current_dir_root() -> Result<PathBuf> {
    find_git_root(&std::env::current_dir()?)
}

/// Extensions that count as source for classification. Vendor/build/output
/// dirs are excluded by path segment, not by extension.
fn language_of(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension()?.to_str()?.to_lowercase();
    Some(match ext.as_str() {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => "typescript",
        "rs" => "rust",
        "go" => "go",
        "py" => "python",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "md" | "mdx" => "markdown",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "json" => "json",
        "sh" | "bash" => "shell",
        _ => return None,
    })
}

fn is_excluded(path: &str) -> bool {
    const EXCLUDED: [&str; 12] = [
        "node_modules/",
        "target/",
        "dist/",
        "build/",
        "vendor/",
        ".git/",
        ".aicontext/",
        ".tgrep/",
        // Tool-managed state is bookkeeping, not product code. Measuring it
        // makes sync self-invalidating: rewriting the state file changes the
        // measured LOC whenever the block changes size.
        ".engineering/",
        "__pycache__/",
        ".venv/",
        "venv/",
    ];
    EXCLUDED.iter().any(|seg| path.contains(seg))
}

fn count_loc(root: &Path, rel: &str) -> u64 {
    let full = root.join(rel);
    let meta = match std::fs::metadata(&full) {
        Ok(m) => m,
        Err(_) => return 0,
    };
    // Skip huge files (likely generated/minified).
    if meta.len() > 1_000_000 {
        return 0;
    }
    match std::fs::read(&full) {
        Ok(bytes) => {
            if bytes.contains(&0) {
                return 0; // binary
            }
            bytes.iter().filter(|&&b| b == b'\n').count() as u64
        }
        Err(_) => 0,
    }
}

fn detect_package_manager(root: &Path) -> Vec<PackageInfo> {
    let mut out = Vec::new();
    let checks: [(&str, &str); 6] = [
        ("bun", "bun.lockb"),
        ("bun", "bun.lock"),
        ("npm", "package-lock.json"),
        ("pnpm", "pnpm-lock.yaml"),
        ("yarn", "yarn.lock"),
        ("cargo", "Cargo.lock"),
    ];
    for (manager, lock) in checks {
        if root.join(lock).exists() {
            out.push(PackageInfo {
                manager: manager.to_string(),
                manifest: lock.to_string(),
                workspaces: detect_workspaces(root, manager),
                scripts: detect_scripts(root),
            });
        }
    }
    // Manifest-only detections (no lockfile yet).
    if out.is_empty() {
        if root.join("package.json").exists() {
            out.push(PackageInfo {
                manager: "npm".to_string(),
                manifest: "package.json".to_string(),
                workspaces: detect_workspaces(root, "npm"),
                scripts: detect_scripts(root),
            });
        } else if root.join("Cargo.toml").exists() {
            out.push(PackageInfo {
                manager: "cargo".to_string(),
                manifest: "Cargo.toml".to_string(),
                workspaces: detect_workspaces(root, "cargo"),
                scripts: Vec::new(),
            });
        } else if root.join("go.mod").exists() {
            out.push(PackageInfo {
                manager: "go".to_string(),
                manifest: "go.mod".to_string(),
                workspaces: Vec::new(),
                scripts: Vec::new(),
            });
        } else if root.join("pyproject.toml").exists() {
            out.push(PackageInfo {
                manager: "python".to_string(),
                manifest: "pyproject.toml".to_string(),
                workspaces: Vec::new(),
                scripts: Vec::new(),
            });
        }
    }
    out
}

fn detect_workspaces(root: &Path, manager: &str) -> Vec<String> {
    match manager {
        "bun" | "npm" | "pnpm" | "yarn" => {
            let text = match std::fs::read_to_string(root.join("package.json")) {
                Ok(t) => t,
                Err(_) => return Vec::new(),
            };
            let v: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => return Vec::new(),
            };
            match v.get("workspaces") {
                Some(serde_json::Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect(),
                Some(serde_json::Value::Object(map)) => map
                    .get("packages")
                    .and_then(|p| p.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            }
        }
        "cargo" => {
            let text = match std::fs::read_to_string(root.join("Cargo.toml")) {
                Ok(t) => t,
                Err(_) => return Vec::new(),
            };
            // Cheap parse: look for [workspace] members without a full TOML model.
            if text.contains("[workspace]") {
                vec!["[workspace]".to_string()]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn detect_scripts(root: &Path) -> Vec<String> {
    let text = match std::fs::read_to_string(root.join("package.json")) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    v.get("scripts")
        .and_then(|s| s.as_object())
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

fn detect_versions(root: &Path) -> Vec<VersionCandidate> {
    let mut out = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
                out.push(VersionCandidate {
                    source: "package.json".to_string(),
                    version: ver.to_string(),
                });
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml")) {
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("version") {
                let ver = rest
                    .trim_start_matches([' ', '=', '"'])
                    .trim_end_matches('"')
                    .trim()
                    .trim_matches('"')
                    .to_string();
                if !ver.is_empty() && ver.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    out.push(VersionCandidate {
                        source: "Cargo.toml".to_string(),
                        version: ver,
                    });
                    break;
                }
            }
        }
    }
    out
}

fn detect_docs(root: &Path) -> Vec<String> {
    let mut docs = Vec::new();
    for candidate in [
        "README.md",
        "docs",
        "docs/architecture",
        "CHANGELOG.md",
        "AGENTS.md",
    ] {
        if root.join(candidate).exists() {
            docs.push(candidate.to_string());
        }
    }
    docs
}

fn detect_ci(root: &Path) -> Vec<String> {
    let mut ci = Vec::new();
    if root.join(".github/workflows").exists() {
        ci.push("github-actions".to_string());
    }
    for f in [
        ".gitlab-ci.yml",
        ".circleci/config.yml",
        "azure-pipelines.yml",
    ] {
        if root.join(f).exists() {
            ci.push(f.to_string());
        }
    }
    ci
}

fn classify(
    source_files: usize,
    loc: u64,
    workspaces: usize,
    languages: usize,
    override_size: Option<&str>,
) -> ComplexityInfo {
    if let Some(size) = override_size {
        let profile = size.to_string();
        return ComplexityInfo {
            profile,
            source_files,
            loc,
            workspaces,
            languages,
            reason: "repository.size override in aicontext.toml".to_string(),
        };
    }
    // Thresholds are configurable in the future; keep deterministic defaults.
    let (profile, reason) = if source_files >= 5000 || loc >= 300_000 || workspaces >= 8 {
        (
            "large",
            format!("{source_files} files / {loc} LOC / {workspaces} workspaces"),
        )
    } else if source_files >= 200 || loc >= 20_000 || workspaces >= 2 {
        (
            "medium",
            format!("{source_files} files / {loc} LOC / {workspaces} workspaces"),
        )
    } else {
        (
            "small",
            format!("{source_files} files / {loc} LOC / {workspaces} workspaces"),
        )
    };
    ComplexityInfo {
        profile: profile.to_string(),
        source_files,
        loc,
        workspaces,
        languages,
        reason,
    }
}

pub fn collect_scan(root: &Path) -> Result<ScanReport> {
    let head = git_output(root, ["rev-parse", "HEAD"]);
    let branch = git_output(root, ["rev-parse", "--abbrev-ref", "HEAD"]);
    let tracked_raw = git_output(root, ["ls-files"]).unwrap_or_default();
    let tracked: Vec<&str> = tracked_raw.lines().collect();

    let mut lang_map: HashMap<String, (usize, u64)> = HashMap::new();
    let mut source_files = 0usize;
    let mut total_loc = 0u64;
    for f in &tracked {
        if is_excluded(f) {
            continue;
        }
        if let Some(lang) = language_of(f) {
            let loc = count_loc(root, f);
            let entry = lang_map.entry(lang.to_string()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += loc;
            source_files += 1;
            total_loc += loc;
        }
    }
    let mut languages: Vec<LanguageStat> = lang_map
        .into_iter()
        .map(|(language, (files, loc))| LanguageStat {
            language,
            files,
            loc,
        })
        .collect();
    languages.sort_by(|a, b| b.files.cmp(&a.files));

    let packages = detect_package_manager(root);
    let commands: Vec<String> = packages
        .iter()
        .flat_map(|p| p.scripts.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let tools_list = tools::detect_core_tools();
    let mut tools_map = HashMap::new();
    for t in tools_list {
        tools_map.insert(t.name.clone(), t);
    }
    let workspace_count: usize = packages.iter().map(|p| p.workspaces.len()).sum();
    let override_size = RepoConfig::load(root).ok().and_then(|c| c.repository.size);
    let complexity = classify(
        source_files,
        total_loc,
        workspace_count,
        languages.len(),
        override_size.as_deref(),
    );

    Ok(ScanReport {
        schema: "aicontext/scan/v1".to_string(),
        git: GitInfo {
            root: root.to_string_lossy().to_string(),
            short_head: head.as_ref().map(|h| h.chars().take(7).collect()),
            head,
            branch,
            tracked_files: tracked.len(),
        },
        languages,
        packages,
        commands,
        tools: tools_map,
        complexity,
        version_candidates: detect_versions(root),
        docs: detect_docs(root),
        ci: detect_ci(root),
    })
}

pub fn cmd_scan(json: bool) -> Result<i32> {
    let root = current_dir_root()?;
    let report = collect_scan(&root)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(0);
    }
    println!("AIContext scan — {}", root.to_string_lossy());
    println!(
        "HEAD: {} (branch {})",
        report.git.short_head.as_deref().unwrap_or("?"),
        report.git.branch.as_deref().unwrap_or("?")
    );
    println!(
        "Tracked files: {} | source files: {} | LOC: ~{}",
        report.git.tracked_files, report.complexity.source_files, report.complexity.loc
    );
    println!(
        "Complexity: {} ({})",
        report.complexity.profile, report.complexity.reason
    );
    if report.languages.is_empty() {
        println!("Languages: none detected");
    } else {
        println!("Languages:");
        for l in &report.languages {
            println!("  - {} ({} files, ~{} LOC)", l.language, l.files, l.loc);
        }
    }
    if report.packages.is_empty() {
        println!("Packages: none detected");
    } else {
        for p in &report.packages {
            println!(
                "Package manager: {} ({}) workspaces={} scripts={}",
                p.manager,
                p.manifest,
                p.workspaces.len(),
                p.scripts.len()
            );
        }
    }
    for v in &report.version_candidates {
        println!("Version: {} from {}", v.version, v.source);
    }
    if !report.docs.is_empty() {
        println!("Docs: {}", report.docs.join(", "));
    }
    if !report.ci.is_empty() {
        println!("CI: {}", report.ci.join(", "));
    }
    let available: Vec<String> = report
        .tools
        .values()
        .filter(|t| t.available)
        .map(|t| format!("{} {}", t.name, t.version.as_deref().unwrap_or("unknown")))
        .collect();
    println!("Tools available: {}", available.join(", "));
    let missing: Vec<String> = report
        .tools
        .values()
        .filter(|t| !t.available)
        .map(|t| t.name.clone())
        .collect();
    if !missing.is_empty() {
        println!("Tools missing: {}", missing.join(", "));
    }
    if report.complexity.profile != "small" {
        println!("Hint: medium/large repo — consider tgrep for text search.");
    }
    if report.complexity.profile == "large" {
        println!("Hint: large repo — CodeGraph is eligible for impact queries.");
    }
    let _ = AiError::new("UNUSED", "placeholder", None);
    Ok(0)
}
