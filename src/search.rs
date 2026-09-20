use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

const MAX_HITS: usize = 30;
const MAX_LINE_CHARS: usize = 300;

#[derive(Debug, Clone, Serialize)]
pub struct TextHit {
    pub path: String,
    pub line: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeHit {
    pub file: String,
    pub line: u64,
    pub text: String,
}

fn truncate(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_LINE_CHARS).collect();
    if s.chars().count() > MAX_LINE_CHARS {
        out.push('…');
    }
    out
}

/// Effective search scope: the normalized path handed to every backend plus
/// the labels reported in the JSON output (present only when the matching
/// flag was used).
struct Scope {
    /// Repo-relative path for the backends; `.` means the whole repository.
    path: String,
    /// `scope` field of `search --json`, present only with `--in`.
    label: Option<String>,
    /// `subproject` field of `search --json`, present only with `--subproject`.
    subproject: Option<String>,
}

/// Normalizes a `--in`/`--subproject` value into a repo-relative scope:
/// backslashes become `/`, leading `./` is stripped, a trailing `/` is
/// dropped and `.`/empty means the whole repository. The result is passed as
/// the trailing path argument to external backends, so absolute paths, `..`
/// segments and leading `-` are rejected instead of being interpreted by
/// grep-likes.
fn normalize_scope(raw: &str) -> Result<String> {
    let mut s = raw.trim().replace('\\', "/");
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    let s = s.trim_end_matches('/').to_string();
    if s.is_empty() || s == "." {
        return Ok(".".to_string());
    }
    let invalid = s.starts_with('/')
        || s.starts_with('-')
        || s.chars().nth(1) == Some(':')
        || s.split('/').any(|seg| seg == "..");
    if invalid {
        return Err(crate::output::AiError::new(
            "INVALID_SCOPE",
            format!("scope `{raw}` must be a repo-relative path inside the repository"),
            None,
        )
        .into());
    }
    Ok(s)
}

/// Resolves `--in`/`--subproject` into the effective scope. `--subproject`
/// is `--in` plus validation: the normalized path must be an exact match of
/// a detected subproject path, otherwise the error lists the candidates. An
/// `--in` path must exist under the repository root.
fn resolve_scope(root: &Path, scope: Option<&str>, subproject: Option<&str>) -> Result<Scope> {
    let (raw, from_subproject) = match (scope, subproject) {
        (_, Some(sub)) => (sub, true),
        (Some(s), None) => (s, false),
        (None, None) => {
            return Ok(Scope {
                path: ".".to_string(),
                label: None,
                subproject: None,
            })
        }
    };
    let normalized = normalize_scope(raw)?;
    if from_subproject {
        let (detected, _) = crate::scan::detect_subprojects(root);
        let candidates: Vec<&str> = detected.iter().map(|s| s.path.as_str()).collect();
        if !candidates.contains(&normalized.as_str()) {
            let list = if candidates.is_empty() {
                "(none detected)".to_string()
            } else {
                candidates.join(", ")
            };
            return Err(crate::output::AiError::new(
                "INVALID_SCOPE",
                format!("`{normalized}` is not a detected subproject; candidates: {list}"),
                Some("aicontext scan --json"),
            )
            .into());
        }
        return Ok(Scope {
            path: normalized.clone(),
            label: Some(normalized.clone()),
            subproject: Some(normalized),
        });
    }
    if !root.join(&normalized).exists() {
        return Err(crate::output::AiError::new(
            "INVALID_SCOPE",
            format!("scope `{normalized}` does not exist under the repository root"),
            None,
        )
        .into());
    }
    Ok(Scope {
        path: normalized.clone(),
        label: Some(normalized),
        subproject: None,
    })
}

/// Parse `path:line:text` lines, tolerating extra colons in text.
fn parse_grep_lines(output: &str) -> Vec<TextHit> {
    let mut hits = Vec::new();
    for line in output.lines() {
        if hits.len() >= MAX_HITS {
            break;
        }
        let mut parts = line.splitn(3, ':');
        let (Some(path), Some(num), Some(text)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let Ok(line) = num.trim().parse::<u64>() else {
            continue;
        };
        if path.is_empty() || line == 0 {
            continue;
        }
        hits.push(TextHit {
            path: path.to_string(),
            line,
            text: truncate(text.trim()),
        });
    }
    hits
}

fn run_backend(root: &Path, program: &str, args: &[String]) -> Option<Vec<TextHit>> {
    // Text backends answer in seconds; 30s keeps `search` bounded on huge repos.
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(root);
    let out = crate::output::command_output(cmd, 30)?;
    // grep-likes exit 1 on no matches — still usable output.
    if out.status.success() || out.status.code() == Some(1) {
        Some(parse_grep_lines(&String::from_utf8_lossy(&out.stdout)))
    } else {
        None
    }
}

/// Literal search with graceful degradation: tgrep -> rg -> git grep.
/// All three backends run in fixed-strings mode so `--text` (the default)
/// is truly literal: a query like `a.b` never matches `axb`. The trailing
/// path argument is the scope, so a scoped search never leaves the subtree.
fn literal_search(root: &Path, scope: &str, query: &str) -> (String, Vec<TextHit>) {
    // Verified against Microsoft tgrep 1.x: `search -n` emits ripgrep-style
    // path:line:text and works without a prebuilt index (uses it when present).
    if crate::tools::detect_tool("tgrep").available {
        let args = [
            "search", "-F", "-n", "--color", "never", "-e", query, "--", scope,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
        if let Some(hits) = run_backend(root, "tgrep", &args) {
            return ("tgrep".to_string(), hits);
        }
    }
    if crate::tools::detect_tool("rg").available {
        let args = [
            "--fixed-strings",
            "--line-number",
            "--no-heading",
            "--color",
            "never",
            "--max-columns",
            "300",
            "--no-messages",
            "-e",
            query,
            scope,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
        if let Some(hits) = run_backend(root, "rg", &args) {
            return ("rg".to_string(), hits);
        }
    }
    let args = [
        "grep",
        "-F",
        "-n",
        "-I",
        "--no-color",
        "-e",
        query,
        "--",
        scope,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect::<Vec<_>>();
    // git grep runs from anywhere inside the repo; anchor at root.
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(&args);
    let out = crate::output::command_output(cmd, 30);
    match out {
        Some(o) if o.status.success() || o.status.code() == Some(1) => (
            "git grep".to_string(),
            parse_grep_lines(&String::from_utf8_lossy(&o.stdout)),
        ),
        _ => ("unavailable".to_string(), Vec::new()),
    }
}

/// Level 1 of progressive disclosure: search persisted knowledge first.
/// Hits are filtered to the scoped path prefix before the 10-hit budget is
/// applied, so a scope that excludes the first file can still surface
/// matches from the second one.
fn knowledge_search(root: &Path, scope: &str, query: &str) -> Vec<KnowledgeHit> {
    let mut hits = Vec::new();
    let files = [".engineering/PROJECT_STATE.md", ".engineering/PATTERNS.md"];
    let needle = query.to_lowercase();
    for rel in files {
        if scope != "." && !rel.starts_with(scope) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if hits.len() >= 10 {
                break;
            }
            if line.to_lowercase().contains(&needle) {
                hits.push(KnowledgeHit {
                    file: rel.to_string(),
                    line: (i + 1) as u64,
                    text: truncate(line.trim()),
                });
            }
        }
    }
    hits
}

fn structural_search(root: &Path, scope: &str, pattern: &str) -> Result<Vec<TextHit>> {
    // Positive condition first: proceed when present, error otherwise.
    if crate::tools::detect_tool("ast-grep").available {
        // present: continue to the search below
    } else {
        return Err(crate::output::AiError::new(
            "MISSING_REQUIRED_TOOL",
            "ast-grep is not installed; --structure needs it",
            Some("aicontext tools plan"),
        )
        .into());
    }
    let mut cmd = Command::new("ast-grep");
    cmd.args(["run", "--pattern", pattern, "--json", scope])
        .current_dir(root);
    let out = crate::output::command_output(cmd, 30).ok_or_else(|| {
        crate::output::AiError::new(
            "TOOL_TIMEOUT",
            "ast-grep did not respond within 30s; --structure needs it healthy",
            Some("retry the search or check the ast-grep installation"),
        )
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Format: [{file, range: {start: {line (0-based)}}}, lines, ...].
    // Be liberal: try a JSON array, then JSON-lines.
    let mut values: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap_or_default();
    if values.is_empty() {
        values = stdout
            .lines()
            .filter_map(|l| serde_json::from_str(l.trim()).ok())
            .collect();
    }
    let mut hits = Vec::new();
    for v in values {
        if hits.len() >= MAX_HITS {
            break;
        }
        let file = v.get("file").and_then(|f| f.as_str()).unwrap_or("?");
        let line = v
            .pointer("/range/start/line")
            .and_then(|l| l.as_u64())
            .map(|l| l + 1)
            .unwrap_or(1);
        let text = v
            .get("lines")
            .and_then(|l| l.as_str())
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .trim();
        hits.push(TextHit {
            path: file.to_string(),
            line,
            text: truncate(text),
        });
    }
    Ok(hits)
}

/// Result of a CodeGraph impact query.
enum Impact {
    /// Graph query ran; hits may be empty when the symbol has no relations.
    Graph(Vec<TextHit>),
    /// Binary present but the query failed (usually: index not built yet).
    NoIndex,
    /// Binary present but wedged: degraded like NoBinary, without blaming the index.
    Timeout,
    /// Binary absent; the caller degrades to text search.
    NoBinary,
}

/// Run `codegraph impact <symbol> --json` (verified against 1.5.0).
/// Read-only: never builds an index; a missing index reports NoIndex.
/// The query runs from the scoped directory so the graph answers for the
/// subproject when it can; a scope the backend cannot serve (missing index,
/// timeout, absent binary) degrades through the same arms as before.
fn codegraph_impact(root: &Path, scope: &str, symbol: &str) -> Impact {
    if crate::tools::detect_tool("codegraph").available {
        let dir = if scope == "." {
            root.to_path_buf()
        } else {
            root.join(scope)
        };
        let mut cmd = Command::new("codegraph");
        cmd.args(["impact", symbol, "--json"]).current_dir(dir);
        // Timeout degrades without blaming the index (see Timeout arm).
        let Some(out) = crate::output::command_output(cmd, 30) else {
            return Impact::Timeout;
        };
        match out {
            o if o.status.success() => {
                let mut hits = Vec::new();
                let parsed: Option<serde_json::Value> =
                    serde_json::from_str(&String::from_utf8_lossy(&o.stdout)).ok();
                if let Some(v) = parsed {
                    if let Some(arr) = v.get("affected").and_then(|a| a.as_array()) {
                        for item in arr {
                            if hits.len() >= MAX_HITS {
                                break;
                            }
                            let path = item.get("filePath").and_then(|x| x.as_str()).unwrap_or("?");
                            let line = item
                                .get("startLine")
                                .and_then(|x| x.as_u64())
                                .unwrap_or(1)
                                .max(1);
                            let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("");
                            let kind = item.get("kind").and_then(|x| x.as_str()).unwrap_or("");
                            hits.push(TextHit {
                                path: path.to_string(),
                                line,
                                text: truncate(&format!("{name} ({kind})")),
                            });
                        }
                    }
                }
                Impact::Graph(hits)
            }
            _ => Impact::NoIndex,
        }
    } else {
        Impact::NoBinary
    }
}

pub fn cmd_search(
    query: String,
    text_only: bool,
    structure: bool,
    impact: bool,
    json: bool,
    scope: Option<String>,
    subproject: Option<String>,
) -> Result<i32> {
    let root = crate::scan::resolve_project_root()?;
    let scope = resolve_scope(&root, scope.as_deref(), subproject.as_deref())?;
    let mode = if impact {
        "impact"
    } else if structure {
        "structure"
    } else {
        "text"
    };
    let _ = text_only;

    let knowledge = knowledge_search(&root, &scope.path, &query);
    let (backend, hits, note) = match mode {
        "structure" => (
            "ast-grep".to_string(),
            structural_search(&root, &scope.path, &query)?,
            None,
        ),
        "impact" => match codegraph_impact(&root, &scope.path, &query) {
            Impact::Graph(hits) => ("codegraph".to_string(), hits, None),
            Impact::NoIndex => {
                let (backend, hits) = literal_search(&root, &scope.path, &query);
                (
                    backend,
                    hits,
                    Some(
                        "codegraph index missing — run `codegraph init` for graph results"
                            .to_string(),
                    ),
                )
            }
            Impact::Timeout | Impact::NoBinary => {
                let (backend, hits) = literal_search(&root, &scope.path, &query);
                (
                    backend,
                    hits,
                    Some("graph unavailable — degraded to text results".to_string()),
                )
            }
        },
        _ => {
            let (b, h) = literal_search(&root, &scope.path, &query);
            (b, h, None)
        }
    };
    if json {
        #[derive(Serialize)]
        struct SearchOut<'a> {
            schema: &'a str,
            query: &'a str,
            mode: &'a str,
            backend: &'a str,
            knowledge: &'a [KnowledgeHit],
            hits: &'a [TextHit],
            #[serde(skip_serializing_if = "Option::is_none")]
            scope: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            subproject: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            note: &'a Option<String>,
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&SearchOut {
                schema: "aicontext/search/v1",
                query: &query,
                mode,
                backend: &backend,
                knowledge: &knowledge,
                hits: &hits,
                scope: &scope.label,
                subproject: &scope.subproject,
                note: &note,
            })?
        );
        return Ok(0);
    }
    if !knowledge.is_empty() {
        println!("Project knowledge:");
        for k in &knowledge {
            println!("  {}:{}: {}", k.file, k.line, k.text);
        }
        println!();
    }
    if hits.is_empty() {
        println!("No matches for {query:?} (backend: {backend}).");
    } else {
        println!("Matches (backend: {backend}):");
        for h in &hits {
            println!("  {}:{}: {}", h.path, h.line, h.text);
        }
    }
    if let Some(n) = note {
        println!("Note: {n}");
    }
    Ok(0)
}
