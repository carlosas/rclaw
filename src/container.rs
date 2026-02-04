use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Instant;
use tracing::{error, info};

#[derive(Debug, Serialize, Deserialize)]
pub struct ContainerInput {
    pub prompt: String,
    pub session_id: Option<String>,
    pub group_folder: String,
    pub chat_jid: String,
    pub is_main: bool,
    pub is_scheduled_task: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContainerOutput {
    pub status: String, // "success" | "error"
    pub result: Option<String>,
    pub new_session_id: Option<String>,
    pub error: Option<String>,
}

pub struct RegisteredGroup {
    pub name: String,
    pub folder: String,
}

pub fn run_container_agent(
    _group: &RegisteredGroup,
    input: &ContainerInput,
) -> Result<ContainerOutput> {
    let start_time = Instant::now();

    info!("Running gemini-cli locally for prompt: {}", input.prompt);

    let mut child = Command::new("gemini")
        .arg("-o")
        .arg("stream-json")
        .arg("--approval-mode")
        .arg("yolo")
        .arg(&input.prompt)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn gemini-cli. Is it installed in the PATH?")?;

    let mut stdout = String::new();
    if let Some(mut stdout_stream) = child.stdout.take() {
        stdout_stream.read_to_string(&mut stdout)?;
    }

    let mut stderr = String::new();
    if let Some(mut stderr_stream) = child.stderr.take() {
        stderr_stream.read_to_string(&mut stderr)?;
    }

    let status = child.wait()?;

    let duration = start_time.elapsed();
    info!("gemini-cli finished in {:?}", duration);

    // Filtrar advertencias de depreciación de Node (punycode bug) y logs de YOLO
    let filtered_stderr = stderr
        .lines()
        .filter(|line| {
            !line.contains("DeprecationWarning")
                && !line.contains("punycode")
                && !line.contains("YOLO mode")
                && !line.contains("Loaded cached credentials")
                && !line.contains("Hook registry")
        })
        .collect::<Vec<_>>()
        .join("\n");

    if !status.success() && !filtered_stderr.trim().is_empty() {
        error!("gemini-cli failed: {}", filtered_stderr);
        return Ok(ContainerOutput {
            status: "error".to_string(),
            result: None,
            new_session_id: None,
            error: Some(format!("Exit code {}: {}", status, filtered_stderr)),
        });
    }

    // Procesar el stream-json preservando el orden cronológico
    let mut combined_output: Vec<String> = Vec::new();

    for line in stdout.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            match val["type"].as_str() {
                Some("message") => {
                    if let Some(content) = val["content"].as_str() {
                        if val["role"].as_str() == Some("assistant") {
                            // Intentar combinar con el último mensaje si es del mismo tipo para evitar saltos excesivos
                            if let Some(last) = combined_output.last_mut() {
                                if !last.starts_with("🔨")
                                    && !last.starts_with("✅")
                                    && !last.starts_with("[RCLAW")
                                {
                                    last.push_str(content);
                                    continue;
                                }
                            }
                            combined_output.push(content.to_string());
                        }
                    }
                }
                Some("tool_use") => {
                    let tool_name = val["tool_name"].as_str().unwrap_or("unknown");
                    let mut tool_desc = tool_name.to_string();
                    if let Some(params) = val["parameters"].as_object() {
                        if let Some(cmd) = params.get("command").and_then(|c| c.as_str()) {
                            tool_desc = format!("{} ({})", tool_name, cmd);
                        }
                    }
                    combined_output.push(format!("[RCLAW_USE_TOOL]{}", tool_desc));
                }
                Some("tool_result") => {
                    if let Some(output) = val["output"].as_str() {
                        combined_output.push(format!("[RCLAW_TOOL_RESULT]{}", output));
                    }
                }
                _ => {}
            }
        }
    }

    let result_content = combined_output.join("\n\n");

    Ok(ContainerOutput {
        status: "success".to_string(),
        result: Some(result_content.trim().to_string()),
        new_session_id: None,
        error: None,
    })
}
