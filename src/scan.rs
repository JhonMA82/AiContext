use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::RepoConfig;
use crate::output::AiError;
use crate::tools::{self, ToolInfo};

/// Versioned contract id for the `scan --json` report. Single source of truth:
/// also used by `tools::empty_report` so the literal never drifts.
pub const SCAN_SCHEMA: &str = "aicontext/scan/v2";

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
    pub subprojects: Vec<Subproject>,
    pub subprojects_source: SubprojectsSource,
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

/// One detected subproject (a workspace member, a container child, or a
/// declared `extra` entry) with its own measured facts. Paths are always
/// repo-relative with `/` separators and the list is sorted by path, so the
/// serialized report is byte-identical across runs.
#[derive(Debug, Clone, Serialize)]
pub struct Subproject {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub manifest: Option<String>,
    pub manager: Option<String>,
    pub commands: Vec<String>,
    pub agents_md: bool,
    pub initialized: bool,
    pub complexity: ComplexityInfo,
}

/// Provenance of the subproject list: how it was resolved and what was
/// rejected instead of guessed.
#[derive(Debug, Clone, Serialize)]
pub struct SubprojectsSource {
    pub mode: String,
    pub declared: Vec<String>,
    pub unresolved: Vec<String>,
    pub containers: Vec<String>,
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
            // Full TOML parse so `[workspace] members` yields the real member
            // paths (the old `[workspace]` sentinel was a placeholder that
            // inflated nothing). The sentinel survives only as a fallback for
            // a manifest whose member list cannot be parsed at all.
            cargo_workspace_members(root).unwrap_or_else(|| {
                let text = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
                if text.contains("[workspace]") {
                    vec!["[workspace]".to_string()]
                } else {
                    Vec::new()
                }
            })
        }
        _ => Vec::new(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct CargoManifest {
    #[serde(default)]
    workspace: Option<CargoWorkspace>,
    #[serde(default)]
    package: Option<CargoPackage>,
}

#[derive(Debug, serde::Deserialize)]
struct CargoWorkspace {
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CargoPackage {
    #[serde(default)]
    name: Option<String>,
}

/// Real `[workspace] members` (minus `exclude`) for a Cargo manifest, or
/// `None` when the file cannot be parsed as TOML at all. A manifest without
/// a `[workspace]` table returns an empty list, not `None`: it is a single
/// crate, not a workspace that failed to parse.
fn cargo_workspace_members(root: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let manifest: CargoManifest = toml::from_str(&text).ok()?;
    let Some(ws) = manifest.workspace else {
        return Some(Vec::new());
    };
    Some(
        ws.members
            .into_iter()
            .filter(|m| !ws.exclude.contains(m))
            .collect(),
    )
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

/// Manifest files that make a directory a subproject, in precedence order.
/// The first match decides `manifest`, `manager` and the subproject `name`.
const SUBPROJECT_MANIFESTS: [&str; 4] = ["package.json", "Cargo.toml", "pyproject.toml", "go.mod"];

/// Default container directories scanned only when nothing is declared.
const DEFAULT_CONTAINERS: [&str; 5] = ["apps", "services", "packages", "libs", "modules"];

/// Collects declared subproject entries from all supported sources, in the
/// documented order: root `package.json` workspaces, `pnpm-workspace.yaml`,
/// `Cargo.toml` workspace members, `go.work` use directives, then the
/// `[subprojects] extra` escape hatch. Entries are returned as authored.
fn declared_subproject_entries(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if root.join("package.json").exists() {
        out.extend(detect_workspaces(root, "npm"));
    }
    if let Ok(text) = std::fs::read_to_string(root.join("pnpm-workspace.yaml")) {
        if let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(&text) {
            if let Some(arr) = v.get("packages").and_then(|p| p.as_sequence()) {
                for item in arr.iter().filter_map(|x| x.as_str()) {
                    out.push(item.to_string());
                }
            }
        }
    }
    if root.join("Cargo.toml").exists() {
        out.extend(detect_workspaces(root, "cargo"));
    }
    if let Ok(text) = std::fs::read_to_string(root.join("go.work")) {
        out.extend(parse_go_work(&text));
    }
    if let Ok(cfg) = RepoConfig::load(root) {
        out.extend(cfg.subprojects.extra);
    }
    out
}

/// Extracts module paths from `go.work` `use` directives, both single-line
/// (`use ./api`) and inside a parenthesized (`use ( ... )`) block. Comments
/// and quotes are stripped; a `=>` replacement keeps only the left side.
fn parse_go_work(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_block = false;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if in_block {
            if line == ")" {
                in_block = false;
            } else if let Some(p) = clean_go_use(line) {
                out.push(p);
            }
            continue;
        }
        let Some(rest) = line.strip_prefix("use") else {
            continue;
        };
        let rest = rest.trim();
        if rest == "(" {
            in_block = true;
        } else if let Some(p) = clean_go_use(rest) {
            out.push(p);
        }
    }
    out
}

fn clean_go_use(line: &str) -> Option<String> {
    let left = line.split("=>").next().unwrap_or(line).trim();
    let path = left.trim_matches('"').trim();
    if path.is_empty() || path == ")" {
        None
    } else {
        Some(path.to_string())
    }
}

/// Normalizes a declared entry into a repo-relative path: strips a leading
/// `./` and any trailing `/`, converts backslashes, and rejects absolute
/// paths or anything that escapes the repo via `..`. Rejection drops the
/// entry rather than guessing at its intent.
fn normalize_entry(entry: &str) -> Option<String> {
    let mut s = entry.trim().replace('\\', "/");
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    let s = s.trim_end_matches('/').to_string();
    if s.is_empty() || s.starts_with('/') || s.chars().nth(1) == Some(':') {
        return None;
    }
    if s.split('/').any(|seg| seg == "..") {
        return None;
    }
    Some(s)
}

/// Expands one declared entry into concrete paths. An exact path is used
/// as-is; a `prefix/*` pattern lists directories under `prefix`. Any other
/// wildcard form (`**`, braces, negation, or `*` outside the last segment)
/// is recorded as unresolved and skipped — the detector never guesses.
fn expand_entry(root: &Path, entry: &str, unresolved: &mut Vec<String>) -> Vec<String> {
    let Some(norm) = normalize_entry(entry) else {
        return Vec::new();
    };
    if norm.contains("**") || norm.contains('{') || norm.contains('}') || norm.contains('!') {
        unresolved.push(entry.to_string());
        return Vec::new();
    }
    match norm.rsplit_once('/') {
        Some((prefix, last)) if last == "*" => {
            let prefix = prefix.trim_end_matches('/');
            let mut out: Vec<String> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(root.join(prefix)) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    let Ok(ft) = e.file_type() else { continue };
                    if ft.is_symlink() || !ft.is_dir() {
                        continue;
                    }
                    out.push(if prefix.is_empty() {
                        name
                    } else {
                        format!("{prefix}/{name}")
                    });
                }
            }
            out
        }
        _ if norm.contains('*') => {
            unresolved.push(entry.to_string());
            Vec::new()
        }
        _ => vec![norm],
    }
}

/// A directory can be a candidate only if it is a real directory (not a
/// symlink), has no hidden path segment, and is not under an excluded
/// vendor/build path.
fn candidate_dir(root: &Path, rel: &str) -> bool {
    if rel.split('/').any(|seg| seg.starts_with('.')) {
        return false;
    }
    if is_excluded(&format!("{rel}/")) {
        return false;
    }
    let Ok(meta) = std::fs::symlink_metadata(root.join(rel)) else {
        return false;
    };
    if meta.file_type().is_symlink() {
        return false;
    }
    meta.is_dir()
}

/// Manager for a subproject based on the manifest that qualified it and the
/// subproject's OWN lockfile. A JS package with no lockfile defaults to npm.
fn detect_subproject_manager(dir: &Path, manifest: &str) -> Option<String> {
    match manifest {
        "package.json" => Some(
            if has_lockfile(dir, "bun.lock") || has_lockfile(dir, "bun.lockb") {
                "bun"
            } else if has_lockfile(dir, "package-lock.json") {
                "npm"
            } else if has_lockfile(dir, "pnpm-lock.yaml") {
                "pnpm"
            } else if has_lockfile(dir, "yarn.lock") {
                "yarn"
            } else {
                "npm"
            }
            .to_string(),
        ),
        "Cargo.toml" => Some("cargo".to_string()),
        "pyproject.toml" => Some("python".to_string()),
        "go.mod" => Some("go".to_string()),
        _ => None,
    }
}

/// True when `name` exists as a file inside `parent` (a subproject dir).
fn has_lockfile(parent: &Path, name: &str) -> bool {
    parent.join(name).is_file()
}

/// Display name: the manifest `name` when the manifest declares one
/// (`package.json`, Cargo `[package] name`), otherwise the directory name.
fn detect_subproject_name(dir: &Path, rel: &str, manifest: Option<&str>) -> String {
    let fallback = rel.rsplit('/').next().unwrap_or(rel).to_string();
    match manifest {
        Some("package.json") => std::fs::read_to_string(dir.join("package.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .unwrap_or(fallback),
        Some("Cargo.toml") => std::fs::read_to_string(dir.join("Cargo.toml"))
            .ok()
            .and_then(|t| toml::from_str::<CargoManifest>(&t).ok())
            .and_then(|m| m.package.and_then(|p| p.name))
            .unwrap_or(fallback),
        _ => fallback,
    }
}

/// Fixed script names from the subproject's own `package.json`, sorted and
/// de-duplicated. Empty for any subproject without one.
fn detect_subproject_commands(dir: &Path) -> Vec<String> {
    let mut cmds = detect_scripts(dir);
    cmds.sort();
    cmds.dedup();
    cmds
}

/// Complexity measured over only the tracked files under `rel`. Workspaces
/// are pinned to 0: the subproject is a single unit, and the repo-wide
/// workspace count is already reported at the root.
fn subproject_complexity(root: &Path, rel: &str, tracked: &[&str]) -> ComplexityInfo {
    let prefix = format!("{rel}/");
    let mut languages: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    let mut source_files = 0usize;
    let mut total_loc = 0u64;
    for f in tracked {
        if !f.starts_with(&prefix) || is_excluded(f) {
            continue;
        }
        if let Some(lang) = language_of(f) {
            languages.insert(lang);
            source_files += 1;
            total_loc += count_loc(root, f);
        }
    }
    classify(source_files, total_loc, 0, languages.len(), None)
}

fn build_subproject(root: &Path, rel: &str, tracked: &[&str]) -> Option<Subproject> {
    let dir = root.join(rel);
    let manifest = SUBPROJECT_MANIFESTS
        .iter()
        .find(|m| dir.join(m).is_file())
        .map(|m| m.to_string());
    let agents_md = dir.join("AGENTS.md").is_file();
    let (kind, manager) = match &manifest {
        Some(m) => ("manifest", detect_subproject_manager(&dir, m)),
        None if agents_md => ("agents-marker", None),
        None => return None,
    };
    Some(Subproject {
        path: rel.to_string(),
        name: detect_subproject_name(&dir, rel, manifest.as_deref()),
        kind: kind.to_string(),
        manifest,
        manager,
        commands: detect_subproject_commands(&dir),
        agents_md,
        initialized: dir.join(".engineering/aicontext.toml").is_file(),
        complexity: subproject_complexity(root, rel, tracked),
    })
}

/// Deterministic subproject detection. Declared workspaces win: when they
/// resolve to at least one usable directory, container directories are never
/// scanned. Otherwise a depth-1 scan of the container directories runs, and
/// if that yields no qualified candidate the mode is `none`. Every ordered
/// output is sorted so the same tree serializes byte-identically.
///
/// Read-only with respect to user-owned files: nothing is written.
pub fn detect_subprojects(root: &Path) -> (Vec<Subproject>, SubprojectsSource) {
    let mut declared: Vec<String> = Vec::new();
    for entry in declared_subproject_entries(root) {
        if !declared.contains(&entry) {
            declared.push(entry);
        }
    }

    let mut unresolved: Vec<String> = Vec::new();
    let mut declared_paths: Vec<String> = Vec::new();
    for entry in &declared {
        for path in expand_entry(root, entry, &mut unresolved) {
            if candidate_dir(root, &path) && !declared_paths.contains(&path) {
                declared_paths.push(path);
            }
        }
    }

    let used_declared = !declared_paths.is_empty();
    let (candidates, scanned_containers) = if used_declared {
        (declared_paths, Vec::new())
    } else {
        let configured = RepoConfig::load(root)
            .map(|c| c.subprojects.containers)
            .unwrap_or_default();
        let containers: Vec<String> = if configured.is_empty() {
            DEFAULT_CONTAINERS.iter().map(|s| s.to_string()).collect()
        } else {
            configured
        };
        let mut scanned: Vec<String> = Vec::new();
        let mut candidates: Vec<String> = Vec::new();
        for container in &containers {
            let Some(name) = normalize_entry(container) else {
                continue;
            };
            if !candidate_dir(root, &name) {
                continue;
            }
            if !scanned.contains(&name) {
                scanned.push(name.clone());
            }
            if let Ok(entries) = std::fs::read_dir(root.join(&name)) {
                for e in entries.flatten() {
                    let child = e.file_name().to_string_lossy().to_string();
                    if child.starts_with('.') {
                        continue;
                    }
                    let Ok(ft) = e.file_type() else { continue };
                    if ft.is_symlink() || !ft.is_dir() {
                        continue;
                    }
                    let path = format!("{name}/{child}");
                    if candidate_dir(root, &path) && !candidates.contains(&path) {
                        candidates.push(path);
                    }
                }
            }
        }
        let mode = if candidates.is_empty() {
            "none"
        } else {
            "containers"
        };
        let scanned = if mode == "containers" {
            scanned
        } else {
            Vec::new()
        };
        (candidates, scanned)
    };

    let tracked_raw = git_output(root, ["ls-files"]).unwrap_or_default();
    let tracked: Vec<&str> = tracked_raw.lines().collect();
    let mut subprojects: Vec<Subproject> = candidates
        .iter()
        .filter_map(|rel| build_subproject(root, rel, &tracked))
        .collect();
    subprojects.sort_by(|a, b| a.path.cmp(&b.path));
    subprojects.dedup_by(|a, b| a.path == b.path);

    // Mode reflects the qualified result: a container scan that finds only
    // directories without a manifest or AGENTS.md marker yields `none`, not
    // `containers`.
    let mode = if used_declared {
        "declared"
    } else if subprojects.is_empty() {
        "none"
    } else {
        "containers"
    };

    declared.sort();
    unresolved.sort();
    unresolved.dedup();
    let mut containers = if mode == "containers" {
        scanned_containers
    } else {
        Vec::new()
    };
    containers.sort();

    (
        subprojects,
        SubprojectsSource {
            mode: mode.to_string(),
            declared,
            unresolved,
            containers,
        },
    )
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
    let (subprojects, subprojects_source) = detect_subprojects(root);
    let mut complexity = classify(
        source_files,
        total_loc,
        workspace_count,
        languages.len(),
        override_size.as_deref(),
    );
    // The router exists only with two or more subprojects; below that the
    // reason text is unchanged. Deterministic count, no extra wording.
    if subprojects.len() >= 2 {
        complexity.reason = format!("{}; {} subprojects", complexity.reason, subprojects.len());
    }

    Ok(ScanReport {
        schema: SCAN_SCHEMA.to_string(),
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
        subprojects,
        subprojects_source,
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
