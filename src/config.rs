use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const REPO_MANIFEST: &str = ".engineering/aicontext.toml";
pub const PROJECT_STATE: &str = ".engineering/PROJECT_STATE.md";
pub const PATTERNS: &str = ".engineering/PATTERNS.md";
pub const CONSISTENCY: &str = ".engineering/consistency.yml";
pub const AST_GREP_RULES: &str = ".engineering/rules/ast-grep";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub schema: String,
    #[serde(default)]
    pub project: ProjectSection,
    #[serde(default)]
    pub state: StateSection,
    #[serde(default)]
    pub version: VersionSection,
    #[serde(default)]
    pub search: SearchSection,
    #[serde(default)]
    pub tools: ToolsSection,
    #[serde(default)]
    pub consistency: ConsistencySection,
    #[serde(default)]
    pub repository: RepositorySection,
    #[serde(default)]
    pub subprojects: SubprojectsSection,
    /// Nested-context marker (WU5). `None` in root manifests; `Some` only
    /// in manifests created by `init --recursive` inside a detected
    /// subproject. Optional, so manifests written before it existed keep
    /// parsing unchanged.
    #[serde(default)]
    pub subproject: Option<SubprojectRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectSection {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_profile")]
    pub profile: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateSection {
    #[serde(default = "default_project_state")]
    pub project_state: String,
    #[serde(default = "default_patterns")]
    pub patterns: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionSection {
    #[serde(default = "default_version_source")]
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchSection {
    #[serde(default = "default_auto")]
    pub text: String,
    #[serde(default = "default_ast_grep")]
    pub structural: String,
    #[serde(default = "default_auto")]
    pub graph: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsSection {
    #[serde(default)]
    pub tgrep: ToolMode,
    #[serde(default)]
    pub codegraph: ToolMode,
    #[serde(default)]
    pub rtk: ToolMode,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolMode {
    #[serde(default = "default_auto")]
    pub mode: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsistencySection {
    #[serde(default = "default_consistency")]
    pub manifest: String,
}

/// Typed view of `.engineering/consistency.yml` (`aicontext/consistency/v1`).
///
/// Semantics enforced by `check`:
/// - `version.projections`: files that must literally contain the version
///   resolved from `version.source` (e.g. `CHANGELOG.md` pins `0.1.0`).
/// - `commands.documented`: command names that must match the detected
///   scripts exactly — no missing entries, no stale ones.
/// - `protected`: paths that must exist (presence gate; content review
///   stays human).
/// - `checks.ast_grep.rules`: directory of ast-grep rule files that `check`
///   executes; any match is a violation.
/// - `adapters.<tool>.policy`: `required` turns that evidence-only adapter
///   finding into a gate (only a clean executed run passes); default is
///   `advisory` (finding always passes, evidence shown in detail).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsistencyFile {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub version: ConsistencyVersion,
    #[serde(default)]
    pub commands: ConsistencyCommands,
    #[serde(default)]
    pub protected: Vec<String>,
    #[serde(default)]
    pub checks: ConsistencyChecks,
    #[serde(default)]
    pub adapters: BTreeMap<String, AdapterPolicy>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsistencyVersion {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub projections: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsistencyCommands {
    #[serde(default)]
    pub documented: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsistencyChecks {
    #[serde(default)]
    pub ast_grep: AstGrepChecks,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AstGrepChecks {
    #[serde(default = "default_ast_grep_rules")]
    pub rules: String,
}

fn default_ast_grep_rules() -> String {
    AST_GREP_RULES.to_string()
}

/// Per-adapter enforcement declared under `adapters:` in consistency.yml.
/// `policy` stays a plain string so `check` can fail closed with a clear
/// message on anything other than `required` / `advisory`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdapterPolicy {
    #[serde(default)]
    pub policy: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepositorySection {
    #[serde(default)]
    pub size: Option<String>,
}

/// Subproject routing declarations. `extra` is the escape hatch for a
/// declared monorepo that still has an undeclared project (e.g. a Rust
/// service next to JS workspaces); entries are added to the declared list
/// verbatim. `containers` overrides the default container directory scan
/// used when nothing is declared. Both default to empty, so manifests
/// written before this section existed keep parsing unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubprojectsSection {
    #[serde(default)]
    pub extra: Vec<String>,
    #[serde(default)]
    pub containers: Vec<String>,
}

/// Pointer from a nested subproject context back to its parent project
/// root. `parent` is the repo-relative path from the nested root upward
/// (`..`, `../..`, …), so tools can navigate without guessing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubprojectRef {
    #[serde(default)]
    pub parent: String,
}

fn default_name() -> String {
    "unknown".to_string()
}
fn default_profile() -> String {
    "auto".to_string()
}
fn default_project_state() -> String {
    PROJECT_STATE.to_string()
}
fn default_patterns() -> String {
    PATTERNS.to_string()
}
fn default_version_source() -> String {
    "package.json".to_string()
}
fn default_auto() -> String {
    "auto".to_string()
}
fn default_ast_grep() -> String {
    "ast-grep".to_string()
}
fn default_consistency() -> String {
    CONSISTENCY.to_string()
}

impl Default for RepoConfig {
    fn default() -> Self {
        Self {
            schema: "aicontext/v1".to_string(),
            project: ProjectSection {
                name: default_name(),
                profile: default_profile(),
            },
            state: StateSection {
                project_state: default_project_state(),
                patterns: default_patterns(),
            },
            version: VersionSection {
                source: default_version_source(),
            },
            search: SearchSection {
                text: default_auto(),
                structural: default_ast_grep(),
                graph: default_auto(),
            },
            tools: ToolsSection::default(),
            consistency: ConsistencySection {
                manifest: default_consistency(),
            },
            repository: RepositorySection { size: None },
            subprojects: SubprojectsSection::default(),
            subproject: None,
        }
    }
}

impl RepoConfig {
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = root.join(REPO_MANIFEST);
        if !path.exists() {
            anyhow::bail!("not initialized: missing {}", REPO_MANIFEST);
        }
        let text = std::fs::read_to_string(&path)?;
        let cfg: Self = toml::from_str(&text)?;
        if cfg.schema != "aicontext/v1" {
            anyhow::bail!("unsupported schema: {}", cfg.schema);
        }
        Ok(cfg)
    }

    pub fn state_path(&self, root: &Path) -> PathBuf {
        root.join(&self.state.project_state)
    }
}

impl ConsistencyFile {
    pub fn load(root: &Path, manifest: &str) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(root.join(manifest))
            .map_err(|_| anyhow::anyhow!("missing {manifest}"))?;
        let file: Self = serde_yaml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("invalid YAML in {manifest}: {e}"))?;
        if file.schema != "aicontext/consistency/v1" {
            anyhow::bail!("unsupported consistency schema: {}", file.schema);
        }
        Ok(file)
    }
}

pub fn minimal_toml(project_name: &str, profile: &str) -> String {
    format!(
        r#"schema = "aicontext/v1"

[project]
name = "{project_name}"
profile = "{profile}"

[state]
project_state = "{PROJECT_STATE}"
patterns = "{PATTERNS}"

[version]
source = "package.json"

[search]
text = "auto"
structural = "ast-grep"
graph = "auto"

[tools.tgrep]
mode = "auto"

[tools.codegraph]
mode = "auto"

[tools.rtk]
mode = "prefer"

[consistency]
manifest = "{CONSISTENCY}"

[subprojects]
extra = []
containers = []
"#
    )
}

/// Version source for a freshly initialized root: the first version file
/// that exists (npm first, then cargo, Go, Python). The root keeps its
/// historical `package.json` default; nested contexts use this so a Rust
/// or Go subproject passes its own `version` gate.
pub fn version_source_for(dir: &Path) -> &'static str {
    for candidate in ["package.json", "Cargo.toml", "go.mod", "pyproject.toml"] {
        if dir.join(candidate).exists() {
            return candidate;
        }
    }
    "package.json"
}

/// Nested-context manifest: the minimal manifest with an explicit version
/// source (so a Rust or Go subproject passes its own `version` gate) and
/// the `[subproject]` parent pointer (`parent_rel` is the repo-relative
/// path from the nested root back to the parent root).
pub fn minimal_nested_toml(
    project_name: &str,
    profile: &str,
    version_source: &str,
    parent_rel: &str,
) -> String {
    let mut out = minimal_toml(project_name, profile).replacen(
        "source = \"package.json\"",
        &format!("source = \"{version_source}\""),
        1,
    );
    out.push_str(&format!("\n[subproject]\nparent = \"{parent_rel}\"\n"));
    out
}

pub fn minimal_consistency(commands: &[String]) -> String {
    let mut sorted = commands.to_vec();
    sorted.sort();
    let documented = if sorted.is_empty() {
        "  documented: []\n".to_string()
    } else {
        let mut s = "  documented:\n".to_string();
        for c in &sorted {
            s.push_str(&format!("    - {c}\n"));
        }
        s
    };
    format!(
        r#"# Consistency declarations (`aicontext check` enforces them).
# - version.projections: files that must contain the resolved version.
# - commands.documented: must match detected scripts exactly (init seeds it).
# - protected: paths that must exist. Empty lists enforce nothing.
# - checks.ast_grep.rules: rule files executed by `check`; a match fails.
schema: aicontext/consistency/v1
version:
  source: package.json
  projections: []
commands:
{documented}protected: []
checks:
  ast_grep:
    rules: {AST_GREP_RULES}
"#
    )
}
