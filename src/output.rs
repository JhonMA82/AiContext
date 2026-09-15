use serde::Serialize;

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

    #[allow(dead_code)]
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
