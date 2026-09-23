//! Engineering Platform compatibility: deterministic detection and
//! consumption of Engineering-managed project contracts.
//!
//! Engineering Platform owns generated project foundations and architectural
//! provenance. AiContext must not rediscover or redefine facts already
//! authoritatively declared by Engineering Platform. When Engineering
//! provenance is absent, AiContext remains fully capable of standalone
//! discovery.
//!
//! Detection is deterministic and version-aware: it consumes only public
//! versioned JSON artifacts (`.engineering/project.json`,
//! `.engineering/project-map.json`, optionally `.engineering/provenance.json`)
//! and never inspects recipe names, Markdown, or LLM output.

use std::path::Path;

/// Engineering metadata directory shared by both projects (ownership is
/// per-file, see README). Reserved for forward-compatible consumers.
/// Keep the literal in sync with `PROJECT_JSON`/`PROJECT_MAP_JSON`.
#[allow(dead_code)]
pub const ENGINEERING_DIR: &str = ".engineering";
/// Authoritative manifest emitted by Engineering Platform.
pub const PROJECT_JSON: &str = ".engineering/project.json";
/// Authoritative surface routing emitted by Engineering Platform.
pub const PROJECT_MAP_JSON: &str = ".engineering/project-map.json";
/// Dated provenance record emitted by Engineering Platform (optional for detection).
pub const PROVENANCE_JSON: &str = ".engineering/provenance.json";

/// Manifest schema versions AiContext understands.
const SUPPORTED_MANIFEST_VERSIONS: [i64; 2] = [1, 2];
/// Project-map schema versions AiContext understands.
const SUPPORTED_MAP_VERSIONS: [i64; 1] = [1];

/// One Engineering-declared surface: authoritative routing fact.
/// `surface` is the logical name (e.g. `web-admin`), `path` the repo-relative
/// destination (e.g. `apps/admin`). Never matched against recipe ids.
/// `provider`/`instructions` are carried for forward-compatible consumers
/// (routing display, AndMar Context) and stay out of the compact projections.
#[derive(Debug, Clone)]
pub struct EngineeringSurface {
    pub surface: String,
    pub path: String,
    #[allow(dead_code)]
    pub provider: String,
    #[allow(dead_code)]
    pub instructions: String,
}

/// Compact Engineering origin facts. References, never copies, the source files.
/// Optional contract fields (`recipe_version`, `catalog_version`,
/// `provenance_present`) are carried for forward-compatible consumers
/// (AndMar Context) and stay out of the compact projections.
#[derive(Debug, Clone)]
pub struct EngineeringInfo {
    pub project: String,
    pub recipe: String,
    #[allow(dead_code)]
    pub recipe_version: Option<String>,
    #[allow(dead_code)]
    pub catalog_version: String,
    pub database_profile: Option<String>,
    pub plan_fingerprint: String,
    pub surfaces: Vec<EngineeringSurface>,
    pub manifest_schema_version: i64,
    pub map_schema_version: i64,
    #[allow(dead_code)]
    pub provenance_present: bool,
}

impl EngineeringInfo {
    /// Repo-relative declared destinations, sorted, deduplicated.
    pub fn declared_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = self.surfaces.iter().map(|s| s.path.clone()).collect();
        out.sort();
        out.dedup();
        out
    }
}

/// Deterministic detection outcome.
#[derive(Debug, Clone)]
pub enum EngineeringDetection {
    /// No Engineering contracts present: standalone repository (first-class).
    Absent,
    /// Engineering-managed with understood contract versions.
    Supported(EngineeringInfo),
    /// Engineering files present but unusable: invalid JSON, missing required
    /// fields, or future schema versions. Never silently treated as standalone.
    /// The recorded versions pinpoint the incompatible contract for operators.
    Unsupported {
        reason: String,
        #[allow(dead_code)]
        manifest_version: Option<String>,
        #[allow(dead_code)]
        map_version: Option<String>,
    },
}

impl EngineeringDetection {
    /// Forward-compatible accessor for the future AndMar Context capability.
    #[allow(dead_code)]
    pub fn info(&self) -> Option<&EngineeringInfo> {
        match self {
            EngineeringDetection::Supported(info) => Some(info),
            _ => None,
        }
    }

    /// Forward-compatible accessor for the future AndMar Context capability.
    #[allow(dead_code)]
    pub fn is_engineering_managed(&self) -> bool {
        matches!(
            self,
            EngineeringDetection::Supported(_) | EngineeringDetection::Unsupported { .. }
        )
    }
}

fn read_json(root: &Path, rel: &str) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(root.join(rel)).ok()?;
    serde_json::from_str(&text).ok()
}

fn schema_version_int(v: &serde_json::Value) -> Option<i64> {
    v.get("schema_version")?.as_i64()
}

fn version_to_string(v: &serde_json::Value) -> Option<String> {
    match v.get("schema_version") {
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn non_empty_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_manifest(
    v: &serde_json::Value,
) -> Option<(
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    i64,
    Vec<(String, String, String, String)>,
)> {
    let schema_version = schema_version_int(v)?;
    let project = non_empty_str(v, "project")?;
    let recipe = non_empty_str(v, "recipe")?;
    let recipe_version = v
        .get("recipe_version")
        .and_then(|x| x.as_str())
        .map(str::to_string);
    let catalog_version = non_empty_str(v, "catalog_version")?;
    let database_profile = v
        .get("database_profile")
        .and_then(|x| x.as_str())
        .map(str::to_string);
    let plan_fingerprint = non_empty_str(v, "plan_fingerprint")?;
    let components = v.get("components")?.as_array()?;
    if components.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for c in components {
        let surface = non_empty_str(c, "surface")?;
        let boilerplate = non_empty_str(c, "boilerplate")?;
        let pin = non_empty_str(c, "pin")?;
        let destination = non_empty_str(c, "destination")?;
        // Reject absolute/escaping destinations: they can never be routing input.
        if destination.starts_with('/') || destination.split('/').any(|s| s == "..") {
            return None;
        }
        out.push((surface, boilerplate, pin, destination));
    }
    Some((
        project,
        recipe,
        recipe_version,
        catalog_version,
        database_profile,
        plan_fingerprint,
        schema_version,
        out,
    ))
}

fn parse_map(v: &serde_json::Value) -> Option<(i64, Vec<EngineeringSurface>)> {
    let schema_version = schema_version_int(v)?;
    let surfaces = v.get("surfaces")?.as_object()?;
    if surfaces.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for (surface, entry) in surfaces {
        let path = non_empty_str(entry, "path")?;
        let provider = non_empty_str(entry, "provider")?;
        let instructions = non_empty_str(entry, "instructions")?;
        if path.starts_with('/') || path.split('/').any(|s| s == "..") {
            return None;
        }
        if surface.trim().is_empty() {
            return None;
        }
        out.push(EngineeringSurface {
            surface: surface.clone(),
            path,
            provider,
            instructions,
        });
    }
    // relationships is required by the contract but may be empty.
    if v.get("relationships").and_then(|r| r.as_array()).is_none() {
        return None;
    }
    out.sort_by(|a, b| a.surface.cmp(&b.surface));
    Some((schema_version, out))
}

/// Deterministic Engineering detection.
///
/// Requires BOTH `project.json` AND `project-map.json` to be present and
/// well-formed. A single `.engineering/` directory is NOT sufficient because
/// AiContext uses the same directory for its own state.
pub fn detect(root: &Path) -> EngineeringDetection {
    let manifest_path = root.join(PROJECT_JSON);
    let map_path = root.join(PROJECT_MAP_JSON);
    let manifest_exists = manifest_path.is_file();
    let map_exists = map_path.is_file();

    if !manifest_exists && !map_exists {
        return EngineeringDetection::Absent;
    }

    // Partial presence is a contract problem, not standalone.
    if !manifest_exists || !map_exists {
        return EngineeringDetection::Unsupported {
            reason: format!(
                "incomplete Engineering metadata: {} present, {} missing — expected both {} and {}",
                if manifest_exists {
                    PROJECT_JSON
                } else {
                    PROJECT_MAP_JSON
                },
                if manifest_exists {
                    PROJECT_MAP_JSON
                } else {
                    PROJECT_JSON
                },
                PROJECT_JSON,
                PROJECT_MAP_JSON
            ),
            manifest_version: manifest_exists
                .then(|| read_json(root, PROJECT_JSON))
                .flatten()
                .as_ref()
                .and_then(version_to_string),
            map_version: map_exists
                .then(|| read_json(root, PROJECT_MAP_JSON))
                .flatten()
                .as_ref()
                .and_then(version_to_string),
        };
    }

    let manifest_raw = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(e) => {
            return EngineeringDetection::Unsupported {
                reason: format!("{PROJECT_JSON} is unreadable ({e})"),
                manifest_version: None,
                map_version: read_json(root, PROJECT_MAP_JSON)
                    .as_ref()
                    .and_then(version_to_string),
            };
        }
    };
    let map_raw = match std::fs::read_to_string(&map_path) {
        Ok(t) => t,
        Err(e) => {
            return EngineeringDetection::Unsupported {
                reason: format!("{PROJECT_MAP_JSON} is unreadable ({e})"),
                manifest_version: serde_json::from_str::<serde_json::Value>(&manifest_raw)
                    .ok()
                    .as_ref()
                    .and_then(version_to_string),
                map_version: None,
            };
        }
    };
    let manifest_value: serde_json::Value = match serde_json::from_str(&manifest_raw) {
        Ok(v) => v,
        Err(e) => {
            return EngineeringDetection::Unsupported {
                reason: format!("{PROJECT_JSON} is not valid JSON ({e})"),
                manifest_version: None,
                map_version: serde_json::from_str::<serde_json::Value>(&map_raw)
                    .ok()
                    .as_ref()
                    .and_then(version_to_string),
            };
        }
    };
    let map_value: serde_json::Value = match serde_json::from_str(&map_raw) {
        Ok(v) => v,
        Err(e) => {
            return EngineeringDetection::Unsupported {
                reason: format!("{PROJECT_MAP_JSON} is not valid JSON ({e})"),
                manifest_version: version_to_string(&manifest_value),
                map_version: None,
            };
        }
    };

    let manifest_version = version_to_string(&manifest_value);
    let map_version = version_to_string(&map_value);

    let Some((
        project,
        recipe,
        recipe_version,
        catalog_version,
        database_profile,
        plan_fingerprint,
        manifest_schema,
        components,
    )) = parse_manifest(&manifest_value)
    else {
        return EngineeringDetection::Unsupported {
            reason: format!(
                "{PROJECT_JSON} is missing required Engineering fields (need schema_version>=1, project, recipe, catalog_version, plan_fingerprint, components[1..] with surface/boilerplate/pin/destination)"
            ),
            manifest_version,
            map_version,
        };
    };
    let Some((map_schema, mut surfaces)) = parse_map(&map_value) else {
        return EngineeringDetection::Unsupported {
            reason: format!(
                "{PROJECT_MAP_JSON} is missing required Engineering fields (need schema_version>=1, surfaces{{path,provider,instructions}} min 1, relationships[])"
            ),
            manifest_version,
            map_version,
        };
    };

    if !SUPPORTED_MANIFEST_VERSIONS.contains(&manifest_schema) {
        return EngineeringDetection::Unsupported {
            reason: format!(
                "{PROJECT_JSON} schema_version {manifest_schema} is not supported (supported: {}) — upgrade aicontext or regenerate with a compatible Engineering Platform",
                SUPPORTED_MANIFEST_VERSIONS
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            manifest_version,
            map_version,
        };
    }
    if !SUPPORTED_MAP_VERSIONS.contains(&map_schema) {
        return EngineeringDetection::Unsupported {
            reason: format!(
                "{PROJECT_MAP_JSON} schema_version {map_schema} is not supported (supported: {}) — upgrade aicontext or regenerate with a compatible Engineering Platform",
                SUPPORTED_MAP_VERSIONS
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            manifest_version,
            map_version,
        };
    }

    // Cross-check: every manifest destination should have a map surface path
    // and vice versa is validated by `check`, not here. Detection only needs
    // both contracts to be individually well-formed. Prefer map surfaces as
    // routing authority; manifest components carry pins/fingerprints.
    let _ = components;
    surfaces.sort_by(|a, b| a.path.cmp(&b.path));

    let provenance_present = root.join(PROVENANCE_JSON).is_file();

    EngineeringDetection::Supported(EngineeringInfo {
        project,
        recipe,
        recipe_version,
        catalog_version,
        database_profile,
        plan_fingerprint,
        surfaces,
        manifest_schema_version: manifest_schema,
        map_schema_version: map_schema,
        provenance_present,
    })
}

/// Advisory one-line origin label for human output.
pub fn origin_label(detection: &EngineeringDetection) -> &'static str {
    match detection {
        EngineeringDetection::Absent => "standalone",
        EngineeringDetection::Supported(_) => "engineering-platform",
        EngineeringDetection::Unsupported { .. } => "engineering-platform (unsupported)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_root(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aicontext-eng-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(dir.join(".engineering")).unwrap();
        dir
    }

    fn write_manifest(root: &Path, schema: i64) {
        let content = format!(
            r#"{{"schema_version":{schema},"project":"demo","recipe":"GP-99","catalog_version":"1.1.0","plan_fingerprint":"abc123","components":[{{"surface":"web","boilerplate":"some-bp","pin":"deadbeef","destination":"apps/web"}}],"files":[]}}"#
        );
        fs::write(root.join(PROJECT_JSON), content).unwrap();
    }

    fn write_map(root: &Path, schema: i64) {
        let content = format!(
            r#"{{"schema_version":{schema},"surfaces":{{"web":{{"path":"apps/web","provider":"some-bp","instructions":"apps/web/AGENTS.md"}} }},"relationships":[]}}"#
        );
        fs::write(root.join(PROJECT_MAP_JSON), content).unwrap();
    }

    #[test]
    fn absent_without_contracts() {
        let dir = tmp_root("absent");
        // .engineering exists but no engineering contracts.
        assert!(matches!(detect(&dir), EngineeringDetection::Absent));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn supported_with_both_contracts() {
        let dir = tmp_root("supported");
        write_manifest(&dir, 2);
        write_map(&dir, 1);
        match detect(&dir) {
            EngineeringDetection::Supported(info) => {
                assert_eq!(info.recipe, "GP-99");
                assert_eq!(info.declared_paths(), vec!["apps/web".to_string()]);
            }
            other => panic!("expected supported, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_partial_presence() {
        let dir = tmp_root("partial");
        write_manifest(&dir, 2);
        assert!(matches!(
            detect(&dir),
            EngineeringDetection::Unsupported { .. }
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_future_manifest() {
        let dir = tmp_root("future");
        write_manifest(&dir, 99);
        write_map(&dir, 1);
        assert!(matches!(
            detect(&dir),
            EngineeringDetection::Unsupported { .. }
        ));
        let _ = fs::remove_dir_all(&dir);
    }
}
