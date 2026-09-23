use serde::Serialize;
use std::io::Read;
use std::process::{Command, Output};
use std::time::Duration;

/// Run a child with a deadline. Reader threads drain piped stdout/stderr so a
/// verbose child can never deadlock the poll loop; on expiry the child is
/// killed and None is returned. Every external process in AIContext goes
/// through here so no command (`status`, `doctor`, `search`, `check`) can
/// hang forever on a wedged binary.
pub fn command_output(mut cmd: Command, secs: u64) -> Option<Output> {
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().ok()?;
    // Take both pipes up front so the reader threads own them and `child`
    // stays whole for try_wait/kill below.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut h) = stdout {
            let _ = h.read_to_end(&mut buf);
        }
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut h) = stderr {
            let _ = h.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = out_thread.join().unwrap_or_default();
                let stderr = err_thread.join().unwrap_or_default();
                return Some(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    // Deliberately not joined: a killed child can leave a
                    // grandchild holding the pipe, and joining would drag the
                    // caller past its deadline. The reader threads exit when
                    // the pipe closes; the caller already has its answer.
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                let _ = child.kill();
                return None;
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("{message}")]
    Check {
        message: String,
        code: String,
        remediation: Option<String>,
    },
}

impl AiError {
    pub fn new(code: &str, message: impl Into<String>, remediation: Option<&str>) -> Self {
        Self::Check {
            code: code.to_string(),
            message: message.into(),
            remediation: remediation.map(|s| s.to_string()),
        }
    }

    pub fn code(&self) -> &str {
        match self {
            Self::Check { code, .. } => code,
        }
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    schema: &'a str,
    code: &'a str,
    message: &'a str,
    remediation: Remediation<'a>,
}

#[derive(Serialize)]
struct Remediation<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<&'a str>,
}

pub fn print_error(err: &anyhow::Error, as_json: bool) {
    if let Some(e) = err.downcast_ref::<AiError>() {
        match e {
            AiError::Check {
                code,
                message,
                remediation,
            } => {
                if as_json {
                    let body = ErrorBody {
                        schema: "aicontext/error/v1",
                        code,
                        message,
                        remediation: Remediation {
                            command: remediation.as_deref(),
                        },
                    };
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&body).unwrap_or_default()
                    );
                } else {
                    eprintln!("error[{code}]: {message}");
                    if let Some(cmd) = remediation {
                        eprintln!("remediation: run `{cmd}`");
                    }
                }
            }
        }
        return;
    }
    if as_json {
        let body = ErrorBody {
            schema: "aicontext/error/v1",
            code: "TOOL_EXECUTION_FAILURE",
            message: &err.to_string(),
            remediation: Remediation { command: None },
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else {
        eprintln!("error: {err:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn command_output_returns_fast_success() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo hi"]);
        let out = command_output(cmd, 10).expect("echo must succeed");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }

    #[test]
    fn command_output_kills_on_deadline() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let start = Instant::now();
        assert!(
            command_output(cmd, 1).is_none(),
            "wedged child must time out"
        );
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "kill must be prompt"
        );
    }
}
