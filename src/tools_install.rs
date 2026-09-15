use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

const REGISTRY_FILE: &str = "managed-tools.json";
const BIN_DIR: &str = "bin";

/// Adapters that are safe global installs via cargo.
const GLOBAL_ADAPTERS: [&str; 2] = ["lychee", "zizmor"];

pub fn managed_prefix() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("USERPROFILE").map(PathBuf::from))
        .context("neither HOME nor USERPROFILE is set")?;
    Ok(home.join(".local").join("share").join("aicontext"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedTool {
    pub name: String,
    pub version: String,
    pub method: String,
    pub path: String,
    pub installed_at: u64,
    pub managed_by: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub tools: Vec<ManagedTool>,
}

pub fn registry_path(prefix: &Path) -> PathBuf {
    prefix.join(REGISTRY_FILE)
}

pub fn load_registry(prefix: &Path) -> Registry {
    std::fs::read_to_string(registry_path(prefix))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_registry(prefix: &Path, registry: &Registry) -> Result<()> {
    std::fs::create_dir_all(prefix)?;
    std::fs::write(
        registry_path(prefix),
        serde_json::to_string_pretty(registry)?,
    )?;
    Ok(())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

enum Method {
    Cargo {
        package: &'static str,
        binary: &'static str,
    },
    Manual {
        instructions: &'static str,
    },
    ProjectLocal {
        note: &'static str,
    },
}

fn method_for(tool: &str) -> Option<Method> {
    match tool {
        "ast-grep" => Some(Method::Cargo {
            package: "ast-grep",
            binary: "ast-grep",
        }),
        "lychee" => Some(Method::Cargo {
            package: "lychee",
            binary: "lychee",
        }),
        "zizmor" => Some(Method::Cargo {
            package: "zizmor",
            binary: "zizmor",
        }),
        "tgrep" => Some(Method::Manual {
            instructions: "brew install tgrep, or build from the official source (cargo install --path tgrep-cli --locked); verify against published checksums",
        }),
        "rtk" => Some(Method::Manual {
            instructions: "install Rust Token Killer from rtk-ai/rtk, verify with `rtk gain`, then enable with `rtk init -g --agent pi`",
        }),
        "codegraph" => Some(Method::Manual {
            instructions: "download the official engine binary + checksum for your platform into the managed prefix",
        }),
        "knip" => Some(Method::ProjectLocal {
            note: "project-local adapter: add as a devDependency in the repo; AIContext never touches manifests",
        }),
        "dependency-cruiser" => Some(Method::ProjectLocal {
            note: "project-local adapter: add as a devDependency in the repo; AIContext never touches manifests",
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
struct Installed {
    tool: String,
    version: String,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct Skipped {
    tool: String,
    reason: String,
}

/// Tools requested by --recommended: missing Recommended entries plus missing
/// global-candidate adapters. Project-local adapters are never installed.
fn recommended_targets(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for section in crate::tools::build_plan(root) {
        for entry in section.entries {
            if entry.state == "missing" {
                if section.title == "Recommended" {
                    out.push(entry.tool);
                } else if section.title == "Project adapters"
                    && GLOBAL_ADAPTERS.contains(&entry.tool.as_str())
                {
                    out.push(entry.tool);
                }
            }
        }
    }
    out
}

fn probe_version(program: &Path) -> Option<String> {
    let out = Command::new(program).arg("--version").output().ok()?;
    if out.status.success() {
        let first = String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if first.is_empty() {
            Some("unknown".to_string())
        } else {
            Some(first)
        }
    } else {
        None
    }
}

fn cargo_present() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn cargo_install(prefix: &Path, package: &str) -> Result<()> {
    let status = Command::new("cargo")
        .env("CARGO_HOME", prefix)
        .args(["install", package, "--locked"])
        .status()
        .context("failed to launch cargo")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("cargo install {package} failed")
    }
}

fn confirm_install(yes: bool, plan_lines: &[String], prefix: &Path) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if std::io::stdin().is_terminal() {
        println!("Installation plan:");
        for line in plan_lines {
            println!("  {line}");
        }
        println!("Target: {} (user prefix, no sudo)", prefix.display());
        print!("Proceed? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        return Ok(answer.trim().eq_ignore_ascii_case("y"));
    }
    println!("Declined: pass --yes or run in a terminal to confirm.");
    Ok(false)
}

pub fn cmd_install(tool: Option<String>, recommended: bool, yes: bool, json: bool) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let requested: Vec<String> = match (tool, recommended) {
        (Some(t), _) => vec![t],
        (None, true) => recommended_targets(&cwd),
        (None, false) => {
            let e = crate::output::AiError::new(
                "INVALID_USAGE",
                "specify a tool or pass --recommended (see `aicontext tools plan`)",
                None,
            );
            crate::output::print_error(&anyhow::anyhow!(e), json);
            return Ok(2);
        }
    };
    for name in &requested {
        if method_for(name).is_none() {
            let e = crate::output::AiError::new(
                "UNKNOWN_TOOL",
                format!("unknown tool '{name}' (see `aicontext tools plan`)"),
                None,
            );
            crate::output::print_error(&anyhow::anyhow!(e), json);
            return Ok(2);
        }
    }

    let mut installed: Vec<Installed> = Vec::new();
    let mut skipped: Vec<Skipped> = Vec::new();
    let mut already_present: Vec<String> = Vec::new();
    let mut installable: Vec<(String, &'static str, &'static str)> = Vec::new();
    for name in &requested {
        if crate::tools::detect_tool(name).available {
            already_present.push(name.clone());
            continue;
        }
        match method_for(name) {
            Some(Method::Cargo { package, binary }) => {
                installable.push((name.clone(), package, binary))
            }
            Some(Method::Manual { instructions }) => skipped.push(Skipped {
                tool: name.clone(),
                reason: format!("no safe managed install; manual steps: {instructions}"),
            }),
            Some(Method::ProjectLocal { note }) => skipped.push(Skipped {
                tool: name.clone(),
                reason: note.to_string(),
            }),
            None => skipped.push(Skipped {
                tool: name.clone(),
                reason: "unknown tool".to_string(),
            }),
        }
    }

    let prefix = managed_prefix()?;
    let mut note: Option<String> = None;
    let mut exit_code = 0;

    if installable.is_empty() {
        if requested.is_empty() {
            note = Some("nothing to install: no recommended tools are missing".to_string());
        }
    } else {
        // Positive conditions first: proceed when cargo exists, fail closed otherwise.
        let have_cargo = cargo_present();
        if have_cargo {
            let plan_lines: Vec<String> = installable
                .iter()
                .map(|(name, package, _)| format!("{name} via cargo install {package} --locked"))
                .collect();
            if confirm_install(yes, &plan_lines, &prefix)? {
                let bin_dir = prefix.join(BIN_DIR);
                std::fs::create_dir_all(&bin_dir)?;
                let mut registry = load_registry(&prefix);
                for (name, package, binary) in &installable {
                    match cargo_install(&prefix, package) {
                        Ok(()) => {
                            let bin_path = bin_dir.join(binary);
                            let version =
                                probe_version(&bin_path).unwrap_or_else(|| "unknown".to_string());
                            registry.tools.retain(|t| t.name != *name);
                            registry.tools.push(ManagedTool {
                                name: name.clone(),
                                version: version.clone(),
                                method: format!("cargo install {package} --locked"),
                                path: bin_path.to_string_lossy().to_string(),
                                installed_at: now_secs(),
                                managed_by: "managed".to_string(),
                            });
                            installed.push(Installed {
                                tool: name.clone(),
                                version,
                                path: bin_path.to_string_lossy().to_string(),
                            });
                        }
                        Err(e) => {
                            skipped.push(Skipped {
                                tool: name.clone(),
                                reason: format!("{e:#}"),
                            });
                            exit_code = 5;
                        }
                    }
                }
                save_registry(&prefix, &registry)?;
                if installed.is_empty() {
                    // Every install failed and was recorded as skipped above.
                } else {
                    note = Some(format!(
                        "installed to {}. Add it to PATH to use the managed tools.",
                        bin_dir.display()
                    ));
                }
            } else {
                note = Some("declined by user; nothing was installed".to_string());
            }
        } else {
            let e = crate::output::AiError::new(
                "MISSING_REQUIRED_TOOL",
                "cargo is required for managed installs but was not found",
                Some("install a Rust toolchain first"),
            );
            crate::output::print_error(&anyhow::anyhow!(e), json);
            return Ok(3);
        }
    }

    if json {
        #[derive(Serialize)]
        struct InstallOut<'a> {
            schema: &'a str,
            requested: &'a [String],
            installed: &'a [Installed],
            skipped: &'a [Skipped],
            already_present: &'a [String],
            #[serde(skip_serializing_if = "Option::is_none")]
            note: &'a Option<String>,
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&InstallOut {
                schema: "aicontext/tools-install/v1",
                requested: &requested,
                installed: &installed,
                skipped: &skipped,
                already_present: &already_present,
                note: &note,
            })?
        );
        return Ok(exit_code);
    }
    if requested.is_empty() {
        println!("Nothing to install.");
    }
    for name in &already_present {
        println!("✓ {name} already present (external, left untouched)");
    }
    for s in &skipped {
        println!("! {} skipped: {}", s.tool, s.reason);
    }
    for i in &installed {
        println!("✓ {} {} at {}", i.tool, i.version, i.path);
    }
    if let Some(n) = note {
        println!("{n}");
    }
    Ok(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(tag: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!("aicontext-reg-{tag}-{}-{n}", std::process::id()))
    }

    #[test]
    fn registry_roundtrips_through_explicit_prefix() {
        let prefix = unique_dir("roundtrip");
        let loaded = load_registry(&prefix);
        assert!(loaded.tools.is_empty());
        let mut reg = Registry::default();
        reg.tools.push(ManagedTool {
            name: "ast-grep".to_string(),
            version: "ast-grep 0.44.1".to_string(),
            method: "cargo install ast-grep --locked".to_string(),
            path: "/tmp/bin/ast-grep".to_string(),
            installed_at: 1,
            managed_by: "managed".to_string(),
        });
        save_registry(&prefix, &reg).expect("save");
        let back = load_registry(&prefix);
        assert_eq!(back.tools.len(), 1);
        assert_eq!(back.tools[0].name, "ast-grep");
        std::fs::remove_dir_all(&prefix).ok();
    }

    #[test]
    fn method_table_covers_plan_surface() {
        for tool in [
            "ast-grep",
            "tgrep",
            "rtk",
            "codegraph",
            "lychee",
            "zizmor",
            "knip",
            "dependency-cruiser",
        ] {
            assert!(method_for(tool).is_some(), "{tool} needs a method");
        }
        assert!(method_for("no-such-tool").is_none());
        for tool in ["knip", "dependency-cruiser"] {
            assert!(
                matches!(method_for(tool), Some(Method::ProjectLocal { .. })),
                "{tool} must stay project-local"
            );
        }
        assert!(GLOBAL_ADAPTERS.contains(&"lychee"));
    }
}
