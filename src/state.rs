use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{
    self, RepoConfig, AST_GREP_RULES, CONSISTENCY, PATTERNS, PROJECT_STATE, REPO_MANIFEST,
};
use crate::output::AiError;
use crate::scan::{self, ScanReport, Subproject};

pub const GEN_START: &str = "<!-- aicontext:generated:start -->";
pub const GEN_END: &str = "<!-- aicontext:generated:end -->";
pub const CUR_START: &str = "<!-- aicontext:curated:start -->";
pub const CUR_END: &str = "<!-- aicontext:curated:end -->";
pub const ROUTING_START: &str = "<!-- aicontext:routing:start -->";
pub const ROUTING_END: &str = "<!-- aicontext:routing:end -->";
pub const CONTEXT_START: &str = "<!-- aicontext:context:start -->";
pub const CONTEXT_END: &str = "<!-- aicontext:context:end -->";

/// The router only exists when there is a choice to route: one project is
/// just a normal repository and gets no `AGENTS.md` block.
const ROUTING_MIN_SUBPROJECTS: usize = 2;
const AGENTS_FILE: &str = "AGENTS.md";
/// Contract id and location of the subproject manifest. WU3 owns seeding,
/// purpose/drift semantics and validation; here it is read-only.
const SUBPROJECTS_SCHEMA: &str = "aicontext/subprojects/v1";
const SUBPROJECTS_FILE: &str = ".engineering/subprojects.yml";
const CELL_MAX_CHARS: usize = 120;
const EMPTY_CELL: &str = "—";

/// Minimal forward-compatible view of `.engineering/subprojects.yml`
/// (`aicontext/subprojects/v1`: path → {purpose, status}). Every field
/// defaults so a partial file still parses; a missing, unreadable or
/// unknown-schema file degrades to `None` instead of failing the render.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SubprojectsFile {
    #[serde(default)]
    pub(crate) schema: String,
    #[serde(default)]
    pub(crate) subprojects: BTreeMap<String, SubprojectEntry>,
}

/// Per-subproject entry. `purpose` feeds the routing table; `status` is
/// declared here for WU3 (adoption lifecycle) and is not rendered yet.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SubprojectEntry {
    #[serde(default)]
    pub(crate) purpose: String,
    #[serde(default)]
    #[allow(dead_code)] // consumed by WU3 (status lifecycle), kept for shape
    pub(crate) status: String,
}

/// Reads the subproject manifest when present and well-formed. Unknown
/// schemas are ignored (forward-compatible), never fatal.
fn load_subprojects_file(root: &Path) -> Option<SubprojectsFile> {
    let text = std::fs::read_to_string(root.join(SUBPROJECTS_FILE)).ok()?;
    let file: SubprojectsFile = serde_yaml::from_str(&text).ok()?;
    if !file.schema.is_empty() && file.schema != SUBPROJECTS_SCHEMA {
        return None;
    }
    Some(file)
}

/// Deterministic markdown table cell: whitespace and newlines collapse to
/// single spaces, `|` is escaped so it cannot split the row, empty values
/// become an em dash and the result is clipped to `CELL_MAX_CHARS`.
fn table_cell(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let escaped = collapsed.replace('|', "\\|");
    if escaped.is_empty() {
        return EMPTY_CELL.to_string();
    }
    escaped.chars().take(CELL_MAX_CHARS).collect()
}

/// The single source of the subproject table (header, separator and rows).
/// Both the `AGENTS.md` routing block and the generated block of
/// `PROJECT_STATE.md` call exactly this function, so the two tables can
/// never drift. Rows are ordered by path and every cell goes through
/// [`table_cell`], which keeps the output byte-stable for `freshness_key`.
fn render_subproject_rows(subprojects: &[Subproject], file: Option<&SubprojectsFile>) -> String {
    let mut ordered: Vec<&Subproject> = subprojects.iter().collect();
    ordered.sort_by(|a, b| a.path.cmp(&b.path));
    let mut out = String::new();
    out.push_str("| Path | Stack | Manifest | Entry context | Commands | Purpose |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for s in ordered {
        let manifest = s
            .manifest
            .as_deref()
            .map(|m| format!("{}/{m}", s.path))
            .unwrap_or_default();
        let entry = if s.agents_md {
            format!("{}/AGENTS.md", s.path)
        } else {
            String::new()
        };
        let purpose = file
            .and_then(|f| f.subprojects.get(&s.path))
            .map(|e| e.purpose.as_str())
            .unwrap_or("");
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            table_cell(&s.path),
            table_cell(s.manager.as_deref().unwrap_or("")),
            table_cell(&manifest),
            table_cell(&entry),
            table_cell(&s.commands.join(", ")),
            table_cell(purpose),
        ));
    }
    out.trim_end_matches('\n').to_string()
}

/// The marked router inserted in the root `AGENTS.md`. Callers enforce
/// [`ROUTING_MIN_SUBPROJECTS`], so a single-project repo never renders one.
fn render_routing_block(subprojects: &[Subproject], file: Option<&SubprojectsFile>) -> String {
    format!(
        "{ROUTING_START}\n\
         ## Subprojects — read exactly one\n\
         \n\
         This repository has {count} subprojects. Do not explore the tree broadly.\n\
         \n\
         {table}\n\
         \n\
         Rules:\n\
         \n\
         - Identify the subproject that owns your task's files and read only its entry context.\n\
         - A file belongs to exactly one subproject; that subproject's `AGENTS.md` governs it.\n\
         - Cross-cutting work: this block plus `.engineering/PROJECT_STATE.md`, nothing else.\n\
         \n\
         > Generated by `aicontext sync`. Do not edit inside this block.\n\
         {ROUTING_END}",
        count = subprojects.len(),
        table = render_subproject_rows(subprojects, file),
    )
}

/// The marked repository-context pointer. The body is exactly the legacy
/// [`crate::agent::AGENTS_POINTER`] text, so a manually migrated file keeps
/// the same visible guidance while gaining the markers.
fn render_context_block() -> String {
    format!(
        "{CONTEXT_START}\n{}{CONTEXT_END}",
        crate::agent::AGENTS_POINTER
    )
}

/// Marker-inclusive slice of `text` for the block delimited by `start` and
/// `end` (both markers must exist, in that order).
fn extract_block<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let s = text.find(start)?;
    let rel = text.get(s..)?.find(end)?;
    Some(&text[s..s + rel + end.len()])
}

/// Where [`upsert_block`] placed (or found) a marked block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// The markers already existed; the block was replaced in place.
    InPlace,
    /// No markers: inserted immediately after the first level-1 heading.
    AfterFirstHeading,
    /// No markers and no heading: inserted at the very top (routing).
    Top,
    /// No markers and no heading: appended at the very end (context pointer).
    End,
}

fn placement_phrase(placement: Placement) -> &'static str {
    match placement {
        Placement::InPlace => "refreshed in place",
        Placement::AfterFirstHeading => "inserted after the first heading",
        Placement::Top => "inserted at the top",
        Placement::End => "appended at the end",
    }
}

/// End byte offset (including the newline, when present) of the first line
/// that is a level-1 ATX heading (`# Something` or `#`), ignoring indent.
/// Only used for placement; heading detection inside fenced code blocks is
/// deliberately not attempted.
fn first_h1_line_end(text: &str) -> Option<usize> {
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("# ") || trimmed.trim() == "#" {
            return Some(offset + line.len());
        }
        offset += line.len();
    }
    None
}

/// Deterministic block placement that never touches a byte outside the
/// marked region: markers present ⇒ replace in place (byte-identical when
/// the render did not change); otherwise insert immediately after the first
/// level-1 heading; otherwise the routing block goes to the very top and
/// the context pointer to the very end.
fn upsert_block(existing: &str, rendered: &str, start: &str, end: &str) -> (String, Placement) {
    if let Some(found) = extract_block(existing, start, end) {
        if found == rendered {
            return (existing.to_string(), Placement::InPlace);
        }
        let s = existing.find(start).unwrap_or(0);
        let e = s + found.len();
        let mut out = String::with_capacity(existing.len() + rendered.len());
        out.push_str(&existing[..s]);
        out.push_str(rendered);
        out.push_str(&existing[e..]);
        return (out, Placement::InPlace);
    }
    if let Some(line_end) = first_h1_line_end(existing) {
        let after = &existing[line_end..];
        // Exactly one blank line around the inserted block without editing
        // the user bytes that follow it.
        let sep = if after.is_empty() || after.starts_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        let mut out = String::with_capacity(existing.len() + rendered.len() + 4);
        out.push_str(&existing[..line_end]);
        out.push('\n');
        out.push_str(rendered);
        out.push_str(sep);
        out.push_str(after);
        return (out, Placement::AfterFirstHeading);
    }
    if start == CONTEXT_START {
        let mut out = existing.to_string();
        if !out.is_empty() {
            out.push('\n');
            if !existing.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str(rendered);
        out.push('\n');
        return (out, Placement::End);
    }
    let mut out = String::with_capacity(existing.len() + rendered.len() + 2);
    out.push_str(rendered);
    out.push_str("\n\n");
    out.push_str(existing);
    (out, Placement::Top)
}

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

/// Deterministic test-function census for the generated block.
///
/// Definition: count of lines whose trimmed content starts with `#[test`
/// in tracked `*.rs` files, split into `src/` (unit) vs `tests/`
/// (integration). The tracked set comes from `git ls-files '*.rs'` run in
/// the repo root (available as `git.root` in the scan report), so untracked
/// files never invalidate freshness. Paths under `.git/`, `target/` or
/// `.engineering/` are skipped (same exclusion spirit as the scan); the
/// list is sorted for determinism and unreadable or oversize (>1MB,
/// likely generated) files are skipped. Only `src/` and `tests/` prefixed
/// paths contribute to the counts.
fn count_test_functions(root: &str) -> (usize, usize) {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "*.rs"])
        .output();
    let out = match out {
        Ok(o) => o,
        Err(_) => return (0, 0),
    };
    if out.status.success() {
    } else {
        return (0, 0);
    }
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut files: Vec<&str> = stdout.lines().collect();
    files.sort();
    let root_path = Path::new(root);
    let mut unit = 0usize;
    let mut integration = 0usize;
    for rel in files {
        if rel.contains(".git/") || rel.contains("target/") || rel.contains(".engineering/") {
            continue;
        }
        let is_unit = rel.starts_with("src/");
        let is_integration = rel.starts_with("tests/");
        if is_unit || is_integration {
        } else {
            continue;
        }
        let full = root_path.join(rel);
        let oversize = match std::fs::metadata(&full) {
            Ok(m) => m.len() > 1_000_000,
            Err(_) => continue,
        };
        if oversize {
            continue;
        }
        let text = match std::fs::read_to_string(&full) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let n = text
            .lines()
            .filter(|l| l.trim_start().starts_with("#[test"))
            .count();
        if is_unit {
            unit += n;
        } else {
            integration += n;
        }
    }
    (unit, integration)
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
    let (unit_tests, integration_tests) = count_test_functions(&report.git.root);
    s.push_str(&format!(
        "Test functions: {} (src: {}, tests: {})\n",
        unit_tests + integration_tests,
        unit_tests,
        integration_tests
    ));
    // Blank line before each list: markdownlint (MD032) inserts one
    // anyway, and any post-sync reformat would invalidate freshness.
    // The generator must emit lint-stable markdown so sync converges.
    s.push_str("\nImportant paths:\n\n");
    for d in &report.docs {
        s.push_str(&format!("- {d}\n"));
    }
    if !report.commands.is_empty() {
        s.push_str("\nCommands:\n\n");
        let mut cmds = report.commands.clone();
        cmds.sort();
        for c in cmds.iter().take(20) {
            s.push_str(&format!("- {c}\n"));
        }
    }
    // Projected subproject facts: `purpose` comes from
    // `.engineering/subprojects.yml`, so editing it is genuine drift that
    // `sync` resolves. Same renderer as the AGENTS.md routing block.
    if report.subprojects.len() >= ROUTING_MIN_SUBPROJECTS {
        let subprojects_file = load_subprojects_file(Path::new(&report.git.root));
        s.push_str("\nSubprojects:\n\n");
        s.push_str(&render_subproject_rows(
            &report.subprojects,
            subprojects_file.as_ref(),
        ));
        s.push_str("\n\n");
    }
    s
}

/// Freshness key for drift comparison: the generated block minus the
/// `Last synchronized commit:` line. That line is provenance (which commit
/// was HEAD when `sync` ran), not a scanned fact — comparing it makes every
/// commit, including the one carrying the synced state, report stale forever.
pub(crate) fn freshness_key(block: &str) -> String {
    block
        .trim()
        .lines()
        .filter(|l| !l.trim_start().starts_with("Last synchronized commit:"))
        .collect::<Vec<_>>()
        .join("\n")
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

/// How `cmd_init`/`cmd_sync` may treat `AGENTS.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutingMode {
    /// `init`: may create the file and insert missing blocks.
    Init,
    /// `sync`: refreshes blocks that already exist; never creates or inserts.
    Sync,
    /// `sync --check`: reports drift only, never writes.
    Report,
}

/// Result of the idempotent `AGENTS.md` routing update (human-readable,
/// printed by `init`/`sync`; the machine contracts stay unchanged).
struct RoutingOutcome {
    note: String,
}

/// Keeps the marked blocks of the root `AGENTS.md` current. The file is
/// user-owned: only bytes between markers are rewritten, `sync` never
/// creates it, and a legacy unmarked pointer is reported, never rewritten.
fn update_agents_routing(
    root: &Path,
    subprojects: &[Subproject],
    file: Option<&SubprojectsFile>,
    mode: RoutingMode,
) -> Result<RoutingOutcome> {
    let path = root.join(AGENTS_FILE);
    let existing = std::fs::read_to_string(&path).ok();

    // One project is not a routing problem: leave any existing block alone
    // (WU4 gates drift later), render nothing, touch no file.
    if subprojects.len() < ROUTING_MIN_SUBPROJECTS {
        let has_block = existing
            .as_deref()
            .map(|t| extract_block(t, ROUTING_START, ROUTING_END).is_some())
            .unwrap_or(false);
        let note = if has_block {
            "AGENTS.md routing block left intact (fewer than 2 subprojects)"
        } else {
            "AGENTS.md untouched: fewer than 2 subprojects"
        };
        return Ok(RoutingOutcome {
            note: note.to_string(),
        });
    }

    let Some(text) = existing else {
        if mode != RoutingMode::Init {
            return Ok(RoutingOutcome {
                note: "AGENTS.md absent: no routing block to refresh".to_string(),
            });
        }
        // Fresh file: routing block first, context pointer last, nothing
        // else. Both go through the same deterministic placement paths.
        let (created, _) = upsert_block("", &render_context_block(), CONTEXT_START, CONTEXT_END);
        let (created, _) = upsert_block(
            &created,
            &render_routing_block(subprojects, file),
            ROUTING_START,
            ROUTING_END,
        );
        std::fs::write(&path, created)?;
        return Ok(RoutingOutcome {
            note: "AGENTS.md created with context and routing blocks".to_string(),
        });
    };

    let write = mode != RoutingMode::Report;
    // Detected before any insertion: the routing block itself mentions
    // `.engineering/PROJECT_STATE.md`, so this must not see our own output.
    let legacy = crate::agent::legacy_pointer_present(&text);
    let mut current = text;
    let mut changed = false;
    let mut notes: Vec<String> = Vec::new();

    // Context pointer: refresh in place when marked; `init` may insert the
    // marked block, except when the legacy unmarked pointer exists — that
    // text is user-owned and must be reported, never rewritten or duplicated.
    if extract_block(&current, CONTEXT_START, CONTEXT_END).is_some() {
        let rendered = render_context_block();
        if extract_block(&current, CONTEXT_START, CONTEXT_END) != Some(rendered.as_str()) {
            if write {
                let (next, _) = upsert_block(&current, &rendered, CONTEXT_START, CONTEXT_END);
                current = next;
                changed = true;
                notes.push("context pointer block refreshed".to_string());
            } else {
                notes.push("context pointer block stale; run `aicontext sync`".to_string());
            }
        }
    } else if legacy {
        notes.push("legacy pointer left untouched (unmarked)".to_string());
    } else if mode == RoutingMode::Init {
        let (next, placement) = upsert_block(
            &current,
            &render_context_block(),
            CONTEXT_START,
            CONTEXT_END,
        );
        if next != current {
            current = next;
            changed = true;
            notes.push(format!(
                "context pointer block {}",
                placement_phrase(placement)
            ));
        }
    } else {
        notes.push("context pointer block missing; run `aicontext init`".to_string());
    }

    // Routing block: refresh when marked; `init` inserts a missing block;
    // `sync` only reports it (WU4 owns drift gates).
    let rendered_routing = render_routing_block(subprojects, file);
    match extract_block(&current, ROUTING_START, ROUTING_END) {
        Some(found) => {
            if found == rendered_routing {
                notes.push("routing block already up to date".to_string());
            } else if write {
                let (next, placement) =
                    upsert_block(&current, &rendered_routing, ROUTING_START, ROUTING_END);
                current = next;
                changed = true;
                notes.push(format!("routing block {}", placement_phrase(placement)));
            } else {
                notes.push("routing block stale; run `aicontext sync`".to_string());
            }
        }
        None if mode == RoutingMode::Init => {
            let (next, placement) =
                upsert_block(&current, &rendered_routing, ROUTING_START, ROUTING_END);
            current = next;
            changed = true;
            notes.push(format!("routing block {}", placement_phrase(placement)));
        }
        None => notes.push("routing block missing; run `aicontext init`".to_string()),
    }

    if changed && write {
        std::fs::write(&path, &current)?;
    }
    Ok(RoutingOutcome {
        note: format!("AGENTS.md: {}", notes.join("; ")),
    })
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
    // Scan first: PROJECT_STATE and consistency.yml are both seeded from
    // detected facts, so a fresh `init` passes `check` and later drift fails.
    let report = scan::collect_scan(&root)?;
    // consistency.yml stub — never overwrite an existing manifest.
    let consistency_path = root.join(CONSISTENCY);
    if !consistency_path.exists() {
        std::fs::write(
            &consistency_path,
            config::minimal_consistency(&report.commands),
        )?;
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

    // The router lives in AGENTS.md (user-owned file): markers only, and
    // only with two or more subprojects. `init` may create the file; `sync`
    // never does.
    let subprojects_file = load_subprojects_file(&root);
    let routing = update_agents_routing(
        &root,
        &report.subprojects,
        subprojects_file.as_ref(),
        RoutingMode::Init,
    )?;

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
        println!("{}", routing.note);
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
    let drift = freshness_key(&current_generated) != freshness_key(&fresh_generated);

    // `sync` refreshes existing AGENTS.md blocks and reports a missing one;
    // it never creates the file. `--check` writes nothing at all.
    let subprojects_file = load_subprojects_file(&root);
    let routing = update_agents_routing(
        &root,
        &report.subprojects,
        subprojects_file.as_ref(),
        if check_only {
            RoutingMode::Report
        } else {
            RoutingMode::Sync
        },
    )?;

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
        } else {
            if drift {
                println!(
                    "stale: PROJECT_STATE generated block differs from scan; run `aicontext sync`."
                );
            } else {
                println!("synchronized.");
            }
            println!("{}", routing.note);
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
    } else {
        if drift {
            println!("Synced generated block in {}", cfg.state.project_state);
        } else {
            println!("Already synchronized (zero diff).");
        }
        println!("{}", routing.note);
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
                .map(|g| freshness_key(&g) != freshness_key(&generated_block(&report)))
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
