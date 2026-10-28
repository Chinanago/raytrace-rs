//! Versioned JSON scripts and a persistent newline-delimited JSON session.
use crate::{Command, Error, Outcome, Request, Result, Session};
use raytrace_io::project::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, Write},
    path::Path,
};

pub const SCRIPT_FORMAT: &str = "raytrace-script-v1";
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Script {
    pub format: String,
    pub commands: Vec<Request>,
}
#[derive(Serialize)]
pub struct ScriptReport {
    pub format: &'static str,
    pub success: bool,
    pub outcomes: Vec<Outcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_command: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}
pub fn run_file(path: &Path) -> Result<ScriptReport> {
    let path = std::fs::canonicalize(path)?;
    let script: Script = serde_json::from_slice(&std::fs::read(&path)?)?;
    let mut session = Session::new(Project::new_assembly())?;
    session.set_base_directory(path.parent().unwrap());
    execute(&mut session, script)
}
pub fn execute(session: &mut Session, script: Script) -> Result<ScriptReport> {
    if script.format != SCRIPT_FORMAT {
        return Err(Error::new("script_version", "未対応のスクリプト形式です"));
    }
    if script.commands.len() > 100_000 {
        return Err(Error::new("script_size", "スクリプトは100,000操作までです"));
    }
    let mut report = ScriptReport {
        format: "raytrace-script-report-v1",
        success: true,
        outcomes: Vec::new(),
        failed_command: None,
        error: None,
    };
    for (i, request) in script.commands.into_iter().enumerate() {
        let waits = matches!(request.command, Command::Wait { .. });
        match session.apply(request) {
            Ok(outcome) => {
                let failed = waits
                    && matches!(
                        outcome.data["status"].as_str(),
                        Some("failed" | "cancelled" | "running")
                    );
                if failed {
                    report.success = false;
                    report.failed_command = Some(i);
                    report.error = Some(Error::new("job_incomplete", outcome.data.to_string()));
                }
                report.outcomes.push(outcome);
                if failed {
                    break;
                }
            }
            Err(e) => {
                report.success = false;
                report.failed_command = Some(i);
                report.error = Some(e);
                break;
            }
        }
    }
    if report.success {
        session.require_finished_jobs()?;
    }
    Ok(report)
}

/// One request and response per line. Model errors leave the session available.
pub fn serve(reader: impl BufRead, mut writer: impl Write) -> Result<()> {
    let mut session = Session::new(Project::new_assembly())?;
    serve_with(reader, &mut writer, |r| session.apply(r))
}
pub fn serve_with(
    reader: impl BufRead,
    mut writer: impl Write,
    mut apply: impl FnMut(Request) -> Result<Outcome>,
) -> Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line)
            .map_err(Error::from)
            .and_then(&mut apply)
        {
            Ok(result) => serde_json::json!({"ok":true,"result":result}),
            Err(error) => serde_json::json!({"ok":false,"error":error}),
        };
        serde_json::to_writer(&mut writer, &response)?;
        writeln!(writer)?;
        writer.flush()?;
    }
    Ok(())
}
