use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aicontext"))
}

fn run<const N: usize>(dir: &Path, args: [&str; N]) -> (i32, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run aicontext");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(255), text)
}

fn cwd() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn completion_emits_per_shell() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let (c, out) = run(&cwd(), ["completion", shell]);
        assert_eq!(c, 0, "{shell} completions must generate: {out}");
        assert!(
            out.contains("aicontext"),
            "{shell} output must mention aicontext"
        );
    }
}

#[test]
fn completion_rejects_unknown_shell() {
    let (c, _) = run(&cwd(), ["completion", "tcsh"]);
    assert!(c != 0, "unknown shell must fail closed, got exit {c}");
}
