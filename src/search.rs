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

/// Preferred graph backend: the codebase-memory-mcp CLI (binary name kept
/// verbatim so tooling probes the real executable).
const CBM: &str = "codebase-memory-mcp";
/// Fallback graph backend, already integrated before CBM existed.
const CODEGRAPH: &str = "codegraph";

/// Wall-clock budget for one backend's graph calls in one `--impact` query.
/// A graph backend answers in seconds on a warm cache but needs tens of
/// seconds per CLI call on a cold 100k-node index, so the budget is shared by
/// every call of one backend: N calls can never add up to an unbounded wait,
/// and each backend gets its own budget so a wedged first backend cannot eat
/// the fallback's time. `AICONTEXT_GRAPH_BUDGET_SECS` (clamped to 1..=600)
/// lowers it for fast-fail environments and offline tests.
struct GraphBudget {
    deadline: std::time::Instant,
}

impl GraphBudget {
    /// Budget for one backend within one query.
    fn new() -> Self {
        let secs = std::env::var("AICONTEXT_GRAPH_BUDGET_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(|s| s.clamp(1, 600))
            .unwrap_or(120);
        Self {
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(secs),
        }
    }

    /// Seconds left for the next call; `None` once the budget is spent.
    fn remaining(&self) -> Option<u64> {
        let left = self
            .deadline
            .saturating_duration_since(std::time::Instant::now());
        if left.as_secs() == 0 {
            None
        } else {
            Some(left.as_secs())
        }
    }
}

/// Result of one graph backend impact query.
enum GraphImpact {
    /// Graph query ran; hits may be empty when the symbol has no relations.
    Ready {
        backend: &'static str,
        hits: Vec<TextHit>,
    },
    /// Backend healthy but this repository (or scope) is not indexed.
    NoIndex { backend: &'static str },
    /// Backend present but wedged: degraded without blaming the index.
    Timeout { backend: &'static str },
    /// Backend absent or disabled; the router degrades to the next one.
    Unavailable { backend: &'static str },
    /// Backend answered but could not resolve the query structurally.
    Invalid { backend: &'static str },
}

impl GraphImpact {
    /// Human reason surfaced in `note` when no graph backend answered.
    fn reason(&self) -> String {
        match self {
            GraphImpact::Ready { .. } => String::new(),
            GraphImpact::NoIndex { backend } if *backend == CBM => {
                format!("{backend}: project not indexed")
            }
            GraphImpact::NoIndex { backend } => {
                format!("{backend} index missing — run `{backend} init` for graph results")
            }
            GraphImpact::Timeout { backend } => format!("{backend}: timed out"),
            GraphImpact::Unavailable { backend } => format!("{backend}: not installed"),
            GraphImpact::Invalid { backend } => {
                format!("{backend}: no structural answer (symbol unresolved or scope not covered)")
            }
        }
    }
}

/// Run `codegraph impact <symbol> --json` (verified against 1.5.0).
/// Read-only: never builds an index; a missing index reports NoIndex.
/// The query runs from the scoped directory so the graph answers for the
/// subproject when it can; a scope the backend cannot serve (missing index,
/// timeout, absent binary) degrades through the same arms as before.
fn codegraph_impact(root: &Path, scope: &str, symbol: &str, budget: &GraphBudget) -> GraphImpact {
    let dir = if scope == "." {
        root.to_path_buf()
    } else {
        root.join(scope)
    };
    let mut cmd = Command::new(CODEGRAPH);
    cmd.args(["impact", symbol, "--json"]).current_dir(dir);
    // Timeout degrades without blaming the index (see Timeout arm).
    let Some(secs) = budget.remaining() else {
        return GraphImpact::Timeout { backend: CODEGRAPH };
    };
    let Some(out) = crate::output::command_output(cmd, secs) else {
        return GraphImpact::Timeout { backend: CODEGRAPH };
    };
    if !out.status.success() {
        return GraphImpact::NoIndex { backend: CODEGRAPH };
    }
    let parsed: Option<serde_json::Value> =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).ok();
    // A successful exit without a parseable answer is not a graph proof.
    let Some(v) = parsed else {
        return GraphImpact::Invalid { backend: CODEGRAPH };
    };
    let mut hits = Vec::new();
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
    GraphImpact::Ready {
        backend: CODEGRAPH,
        hits,
    }
}

/// Answer of one `codebase-memory-mcp cli <tool>` call.
enum CbmAnswer {
    /// Tool answered with a JSON document (an `error` envelope counts as
    /// `Failed`: the tool ran but did not serve the request).
    Json(serde_json::Value),
    /// Non-zero exit, error envelope or unparsable output.
    Failed,
    /// The process outlived its bounded timeout.
    Timeout,
}

/// One read-only `codebase-memory-mcp cli <tool> ... --format json` call.
/// Verified against the codebase-memory-mcp 0.11.0 CLI (`cli --help`): every
/// tool takes `--format json` and answers a single JSON document. Nothing in
/// this adapter indexes: `index_repository` is never invoked, so `search` has
/// no indexing side effect.
fn cbm_cli(args: &[&str], budget: &GraphBudget) -> CbmAnswer {
    let mut cmd = Command::new(CBM);
    cmd.arg("cli").args(args);
    // Graph queries stay bounded by the shared budget; a wedged backend
    // degrades instead of hanging.
    let Some(secs) = budget.remaining() else {
        return CbmAnswer::Timeout;
    };
    let Some(out) = crate::output::command_output(cmd, secs) else {
        return CbmAnswer::Timeout;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    // The CLI may print warnings before the document (e.g. deprecation
    // notices): parse the first JSON value instead of the whole buffer.
    let value = text
        .find('{')
        .and_then(|start| {
            serde_json::Deserializer::from_str(&text[start..])
                .into_iter::<serde_json::Value>()
                .next()
        })
        .and_then(|r| r.ok());
    let Some(value) = value else {
        return CbmAnswer::Failed;
    };
    if value.get("error").is_some() {
        return CbmAnswer::Failed;
    }
    CbmAnswer::Json(value)
}

/// A node of the codebase-memory graph: enough to emit one `TextHit`.
struct CbmNode {
    name: String,
    qn: String,
    label: String,
    file: String,
    line: u64,
}

/// Start line of a `lines` cell such as `105-185` (single numbers pass).
fn start_line(cell: &str) -> u64 {
    cell.split('-')
        .next()
        .unwrap_or("")
        .trim()
        .parse::<u64>()
        .unwrap_or(1)
        .max(1)
}

/// Parse a `search_graph` answer into nodes. Verified shapes (0.11.0):
/// table `{"cols":["qn","label","file","lines","rank"],"rows":[[...]]}` and
/// grouped `{"cols":["name","label","lines","in","out"],"groups":[{"qn_prefix","file","rows":[[...]]}]}`.
/// Columns are located by name, so a column order change stays harmless.
fn parse_graph_nodes(value: &serde_json::Value) -> Vec<CbmNode> {
    let mut nodes = Vec::new();
    let cell = |cols: &[serde_json::Value], row: &[serde_json::Value], name: &str| -> String {
        cols.iter()
            .position(|c| c.as_str() == Some(name))
            .and_then(|i| row.get(i))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let cols: Vec<serde_json::Value> = value
        .get("cols")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    if let Some(rows) = value.get("rows").and_then(|r| r.as_array()) {
        for row in rows.iter().filter_map(|r| r.as_array()) {
            let name = cell(&cols, row, "name");
            let qn = cell(&cols, row, "qn");
            nodes.push(CbmNode {
                name: if name.is_empty() {
                    qn.rsplit('.').next().unwrap_or("").to_string()
                } else {
                    name
                },
                qn,
                label: cell(&cols, row, "label"),
                file: cell(&cols, row, "file"),
                line: start_line(&cell(&cols, row, "lines")),
            });
        }
    }
    if let Some(groups) = value.get("groups").and_then(|g| g.as_array()) {
        for group in groups {
            let file = group
                .get("file")
                .and_then(|f| f.as_str())
                .unwrap_or("")
                .to_string();
            let prefix = group
                .get("qn_prefix")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let group_cols: Vec<serde_json::Value> = group
                .get("cols")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_else(|| cols.clone());
            for row in group
                .get("rows")
                .and_then(|r| r.as_array())
                .into_iter()
                .flatten()
                .filter_map(|r| r.as_array())
            {
                let name = cell(&group_cols, row, "name");
                // qn_rule (verified): `qn = qn_prefix == "" ? name : qn_prefix + "." + name`.
                let qn = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                nodes.push(CbmNode {
                    name,
                    qn,
                    label: cell(&group_cols, row, "label"),
                    file: file.clone(),
                    line: start_line(&cell(&group_cols, row, "lines")),
                });
            }
        }
    }
    nodes
}

/// Caller/callee qualified names of a `trace_path` answer (verified shape:
/// `callers`/`callees` objects with `groups[].rows[]`, columns `name`,`hop`),
/// paired with their direction. The group `qn_prefix` is part of the identity:
/// a bare `name` would match unrelated same-named nodes across the repository.
fn parse_trace_qns(value: &serde_json::Value) -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = Vec::new();
    for (key, role) in [("callers", "caller"), ("callees", "callee")] {
        let Some(section) = value.get(key) else {
            continue;
        };
        let cols: Vec<serde_json::Value> = section
            .get("cols")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        let name_idx = cols
            .iter()
            .position(|c| c.as_str() == Some("name"))
            .unwrap_or(0);
        for group in section
            .get("groups")
            .and_then(|g| g.as_array())
            .into_iter()
            .flatten()
        {
            // qn_rule (verified): `qn = qn_prefix == "" ? name : qn_prefix + "." + name`.
            let prefix = group
                .get("qn_prefix")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            for row in group
                .get("rows")
                .and_then(|r| r.as_array())
                .into_iter()
                .flatten()
                .filter_map(|r| r.as_array())
            {
                let Some(name) = row.get(name_idx).and_then(|n| n.as_str()) else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let qn = if prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{prefix}.{name}")
                };
                out.push((qn, role));
            }
        }
    }
    out
}

/// Escape a literal so it can be embedded in a `--name-pattern`/`--qn-pattern`
/// regex.
fn regex_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if r"\.^$|?*+()[]{}".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Exact-match pattern for one or more literals: `^(a|b)$`. Names and
/// qualified names coming from a backend answer are escaped, so the pattern
/// never interprets them.
fn exact_name_pattern(names: &[String]) -> String {
    let alts: Vec<String> = names.iter().map(|n| regex_literal(n)).collect();
    format!("^({})$", alts.join("|"))
}

/// Whether a repo-relative hit path falls inside the effective scope.
fn in_scope(path: &str, scope: &str) -> bool {
    scope == "." || path == scope || path.starts_with(&format!("{scope}/"))
}

/// `check_index_coverage` can serve the scope only when it recorded no issue
/// for it and the path is not simply absent from the index. Verified fields
/// (0.11.0): `paths[].{path,status,freshness}` plus an explicit best-effort
/// caveat, so anything ambiguous degrades instead of trusting the graph.
fn coverage_serves_scope(value: &serde_json::Value, scope: &str) -> bool {
    let rows = value
        .get("paths")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    rows.iter().any(|row| {
        let path = row.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let status = row.get("status").and_then(|s| s.as_str()).unwrap_or("");
        let freshness = row.get("freshness").and_then(|f| f.as_str()).unwrap_or("");
        path == scope && status == "no_recorded_issue" && freshness != "missing"
    })
}

/// Project name whose `root_path` is exactly this repository root. Both sides
/// are canonicalized before comparing; a same-basename project elsewhere is
/// never accepted, and no basename inference is attempted.
fn find_project(value: &serde_json::Value, root: &Path) -> Option<String> {
    let want = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    value
        .get("projects")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter(|p| {
            let got = p.get("root_path").and_then(|r| r.as_str()).unwrap_or("");
            std::fs::canonicalize(got)
                .map(|g| g == want)
                .unwrap_or_else(|_| got == want.to_string_lossy())
        })
        .find_map(|p| {
            p.get("name")
                .and_then(|n| n.as_str())
                .map(|n| n.to_string())
        })
}

/// Read-only codebase-memory-mcp impact adapter.
///
/// Pipeline (verified against the 0.11.0 CLI):
/// `list_projects` (exact root match) -> `index_status` (index ready for that
/// exact root) -> `check_index_coverage` (scoped queries only) ->
/// `search_graph --name-pattern` (resolve the symbol) ->
/// `trace_path --direction both` (its relations). Five CLI calls at most,
/// small limits, one shared time budget, no generated Cypher and no indexing
/// side effect.
fn cbm_impact(root: &Path, scope: &str, symbol: &str, budget: &GraphBudget) -> GraphImpact {
    // Every CLI call below shares one budget (see `GraphBudget`).
    let cli = |args: &[&str]| cbm_cli(args, budget);
    // 1. Project discovery: exact root_path match over the project list.
    let mut project = None;
    for page in 0..5 {
        let offset = (page * 50).to_string();
        match cli(&[
            "list_projects",
            "--format",
            "json",
            "--limit",
            "50",
            "--offset",
            &offset,
        ]) {
            CbmAnswer::Timeout => return GraphImpact::Timeout { backend: CBM },
            CbmAnswer::Failed => return GraphImpact::Invalid { backend: CBM },
            CbmAnswer::Json(v) => {
                if let Some(name) = find_project(&v, root) {
                    project = Some(name);
                    break;
                }
                let more = v.get("has_more").and_then(|h| h.as_bool()).unwrap_or(false);
                if !more {
                    break;
                }
            }
        }
    }
    // No index for this exact root: never index as a side effect of search.
    let Some(project) = project else {
        return GraphImpact::NoIndex { backend: CBM };
    };

    // 2. Index health, re-checking the root as defense in depth.
    match cli(&["index_status", "--project", &project, "--format", "json"]) {
        CbmAnswer::Timeout => return GraphImpact::Timeout { backend: CBM },
        CbmAnswer::Failed => return GraphImpact::NoIndex { backend: CBM },
        CbmAnswer::Json(v) => {
            let ready = v.get("status").and_then(|s| s.as_str()) == Some("ready");
            let same_root = v
                .get("root_path")
                .and_then(|r| r.as_str())
                .map(|p| std::fs::canonicalize(p).ok())
                .flatten()
                .map(|p| p == std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()))
                .unwrap_or(false);
            if !ready || !same_root {
                return GraphImpact::NoIndex { backend: CBM };
            }
        }
    }

    // 3. Scoped queries must prove coverage of the scope, or degrade instead
    //    of returning repo-wide hits labelled as scoped.
    if scope != "." {
        match cli(&[
            "check_index_coverage",
            "--project",
            &project,
            "--paths",
            scope,
            "--format",
            "json",
        ]) {
            CbmAnswer::Timeout => return GraphImpact::Timeout { backend: CBM },
            CbmAnswer::Failed => return GraphImpact::Invalid { backend: CBM },
            CbmAnswer::Json(v) => {
                if !coverage_serves_scope(&v, scope) {
                    return GraphImpact::Invalid { backend: CBM };
                }
            }
        }
    }

    // 4. Resolve the symbol structurally: exact name match only. A semantic
    //    match is never presented as a proven blast radius.
    let pattern = exact_name_pattern(&[symbol.to_string()]);
    let nodes = match cli(&[
        "search_graph",
        "--project",
        &project,
        "--name-pattern",
        &pattern,
        "--format",
        "json",
        "--limit",
        "5",
    ]) {
        CbmAnswer::Timeout => return GraphImpact::Timeout { backend: CBM },
        CbmAnswer::Failed => return GraphImpact::Invalid { backend: CBM },
        CbmAnswer::Json(v) => parse_graph_nodes(&v),
    };
    let mut symbols: Vec<CbmNode> = nodes
        .into_iter()
        .filter(|n| n.name == symbol || n.qn == symbol)
        .collect();
    symbols.sort_by(|a, b| (&a.file, a.line, &a.qn).cmp(&(&b.file, b.line, &b.qn)));
    if symbols.is_empty() {
        return GraphImpact::Invalid { backend: CBM };
    }

    // 5. Relations (direct callers and callees = blast radius, depth 1).
    let anchor = &symbols[0];
    let relations = match cli(&[
        "trace_path",
        "--function-name",
        &anchor.qn,
        "--project",
        &project,
        "--direction",
        "both",
        "--depth",
        "1",
        "--limit",
        "20",
        "--format",
        "json",
    ]) {
        CbmAnswer::Timeout => return GraphImpact::Timeout { backend: CBM },
        // Ambiguous or unknown target: CBM cannot resolve it structurally.
        CbmAnswer::Failed => return GraphImpact::Invalid { backend: CBM },
        CbmAnswer::Json(v) => parse_trace_qns(&v),
    };

    // 6. One batched lookup resolves file/line for every related node, keyed
    //    by qualified name so a bare `main` never drags in unrelated nodes.
    let mut roles: std::collections::BTreeMap<String, Vec<&'static str>> = Default::default();
    for (qn, role) in relations {
        let entry = roles.entry(qn).or_default();
        if !entry.contains(&role) {
            entry.push(role);
        }
    }
    let mut related: Vec<CbmNode> = Vec::new();
    if !roles.is_empty() {
        let qns: Vec<String> = roles.keys().cloned().collect();
        let pattern = exact_name_pattern(&qns);
        related = match cli(&[
            "search_graph",
            "--project",
            &project,
            "--qn-pattern",
            &pattern,
            "--format",
            "json",
            "--limit",
            "20",
        ]) {
            CbmAnswer::Timeout => return GraphImpact::Timeout { backend: CBM },
            // Unresolvable relations: keep the symbol hits, drop the names.
            CbmAnswer::Failed => Vec::new(),
            CbmAnswer::Json(v) => parse_graph_nodes(&v)
                .into_iter()
                .filter(|n| roles.contains_key(&n.qn))
                .collect(),
        };
    }

    // 7. Emit hits in a byte-stable order, filtered to the effective scope and
    //    capped like every other backend. A node both queried and related keeps
    //    one hit with every role it plays.
    let mut merged: std::collections::BTreeMap<String, (&CbmNode, Vec<&'static str>)> =
        Default::default();
    for node in symbols.iter().chain(related.iter()) {
        if !in_scope(&node.file, scope) {
            continue;
        }
        let entry = merged.entry(node.qn.clone()).or_insert((node, Vec::new()));
        for role in roles.get(&node.qn).into_iter().flatten().copied() {
            if !entry.1.contains(&role) {
                entry.1.push(role);
            }
        }
    }
    let mut hits: Vec<TextHit> = Vec::new();
    for (node, kinds) in merged.into_values() {
        let mut kinds = kinds;
        kinds.sort_unstable();
        let text = if kinds.is_empty() {
            format!("{} ({})", node.name, node.label)
        } else {
            format!("{} ({}, {})", node.name, node.label, kinds.join("+"))
        };
        hits.push(TextHit {
            path: node.file.clone(),
            line: node.line,
            text: truncate(&text),
        });
    }
    hits.sort_by(|a, b| (&a.path, a.line, &a.text).cmp(&(&b.path, b.line, &b.text)));
    hits.truncate(MAX_HITS);
    // Empty-but-valid: a symbol with no in-scope relations is still Ready.
    GraphImpact::Ready { backend: CBM, hits }
}

/// Outcome of the graph router.
enum GraphRoute {
    /// First healthy backend answered; no second backend runs.
    Ready {
        backend: &'static str,
        hits: Vec<TextHit>,
    },
    /// No graph backend could answer: the caller falls back to text search.
    Degrade { notes: Vec<String> },
}

/// Graph router: codebase-memory-mcp first, CodeGraph as fallback, text
/// degradation left to the caller. The first `Ready` is terminal — a valid
/// empty graph answer never triggers a second backend — and the two graph
/// backends never run in parallel for one query.
fn route_impact(root: &Path, scope: &str, symbol: &str) -> GraphRoute {
    let cfg = crate::config::RepoConfig::load(root).ok();
    let mut notes = Vec::new();
    for backend in [CBM, CODEGRAPH] {
        match crate::tools::tool_state(cfg.as_ref(), backend) {
            crate::tools::ToolState::Disabled => {
                notes.push(format!("{backend}: disabled in aicontext.toml"));
                continue;
            }
            crate::tools::ToolState::Missing => {
                notes.push(GraphImpact::Unavailable { backend }.reason());
                continue;
            }
            crate::tools::ToolState::Usable => {}
        }
        // Fresh budget per backend: a wedged backend never starves the next.
        let budget = GraphBudget::new();
        let impact = if backend == CBM {
            cbm_impact(root, scope, symbol, &budget)
        } else {
            codegraph_impact(root, scope, symbol, &budget)
        };
        match impact {
            GraphImpact::Ready { backend, hits } => return GraphRoute::Ready { backend, hits },
            other => notes.push(other.reason()),
        }
    }
    GraphRoute::Degrade { notes }
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
        "impact" => match route_impact(&root, &scope.path, &query) {
            GraphRoute::Ready { backend, hits } => (backend.to_string(), hits, None),
            GraphRoute::Degrade { notes } => {
                let (backend, hits) = literal_search(&root, &scope.path, &query);
                (
                    backend,
                    hits,
                    Some(format!(
                        "graph unavailable — degraded to text results ({})",
                        notes.join("; ")
                    )),
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
