use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepositorySection {
    #[serde(default)]
    pub size: Option<String>,
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
"#
    )
}

pub fn minimal_consistency() -> String {
    format!(
        r#"schema: aicontext/consistency/v1
version:
  source: package.json
  projections: []
commands:
  documented: []
protected: []
checks:
  ast_grep:
    rules: {AST_GREP_RULES}
"#
    )
}
