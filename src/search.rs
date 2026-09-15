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

/// Parse `path:line:text` lines, tolerating extra colons in text.
fn parse_grep_lines(output: &str) -> Vec<TextHit> {
    let mut hits = Vec::new();
    for line in output.lines() {
        if hits.len() >= MAX_HITS {
            break;
        }
        let mut parts = line.splitn(3, ':');
        let (Some(path), Some(num), Some(text)) = (parts.next(), parts.next(), parts.next())
        else {
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
    let out = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    // grep-likes exit 1 on no matches — still usable output.
    if out.status.success() || out.status.code() == Some(1) {
        Some(parse_grep_lines(&String::from_utf8_lossy(&out.stdout)))
    } else {
        None
    }
}

/// Literal search with graceful degradation: tgrep -> rg -> git grep.
/// Returns the backend name and hits.
fn literal_search(root: &Path, query: &str) -> (String, Vec<TextHit>) {
    if crate::tools::detect_tool("tgrep").available {
        let args = vec![query.to_string(), ".".to_string()];
        if let Some(hits) = run_backend(root, "tgrep", &args) {
            return ("tgrep".to_string(), hits);
        }
    }
    if crate::tools::detect_tool("rg").available {
        let args = [
            "--line-number",
            "--no-heading",
            "--color",
            "never",
            "--max-columns",
            "300",
            "--no-messages",
            "-e",
            query,
            ".",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
        if let Some(hits) = run_backend(root, "rg", &args) {
            return ("rg".to_string(), hits);
        }
    }
    let args = [
        "grep", "-n", "-I", "--no-color", "-e", query, "--", ".",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect::<Vec<_>>();
    // git grep runs from anywhere inside the repo; anchor at root.
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(&args)
        .output();
    match out {
        Ok(o) if o.status.success() || o.status.code() == Some(1) => (
            "git grep".to_string(),
            parse_grep_lines(&String::from_utf8_lossy(&o.stdout)),
        ),
        _ => ("unavailable".to_string(), Vec::new()),
    }
}

/// Level 1 of progressive disclosure: search persisted knowledge first.
fn knowledge_search(root: &Path, query: &str) -> Vec<KnowledgeHit> {
    let mut hits = Vec::new();
    let files = [
        ".engineering/PROJECT_STATE.md",
        ".engineering/PATTERNS.md",
    ];
    let needle = query.to_lowercase();
    for rel in files {
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

fn structural_search(root: &Path, pattern: &str) -> Result<Vec<TextHit>> {
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
    let out = Command::new("ast-grep")
        .args(["run", "--pattern", pattern, "--json", "."])
        .current_dir(root)
        .output()?;
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

pub fn cmd_search(
    query: String,
    text_only: bool,
    structure: bool,
    impact: bool,
    json: bool,
) -> Result<i32> {
    let root = crate::scan::current_dir_root()?;
    let mode = if impact {
        "impact"
    } else if structure {
        "structure"
    } else {
        "text"
    };
    let _ = text_only;

    let knowledge = knowledge_search(&root, &query);
    let (backend, mut hits, note) = match mode {
        "structure" => ("ast-grep".to_string(), structural_search(&root, &query)?, None),
        "impact" => {
            if crate::tools::detect_tool("codegraph").available {
                (
                    "codegraph".to_string(),
                    Vec::new(),
                    Some(
                        "codegraph relations are not wired in P0; showing text results"
                            .to_string(),
                    ),
                )
            } else {
                let (b, h) = literal_search(&root, &query);
                (
                    b,
                    h,
                    Some("graph unavailable — degraded to text results".to_string()),
                )
            }
        }
        _ => {
            let (b, h) = literal_search(&root, &query);
            (b, h, None)
        }
    };
    // Impact with codegraph present still benefits from text hits in P0.
    if mode == "impact" && crate::tools::detect_tool("codegraph").available {
        let (_, h) = literal_search(&root, &query);
        hits = h;
    }

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
