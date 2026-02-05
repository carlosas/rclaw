use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Instant;
use tracing::{error, info, debug};
use std::fs;

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
    group: &RegisteredGroup,
    input: &ContainerInput,
) -> Result<ContainerOutput> {
    let start_time = Instant::now();
    let project_root = std::env::current_dir().context("Failed to get current dir")?;
    let home_dir = dirs::home_dir().context("Failed to get home dir")?;

    info!("Running rclaw-agent container for prompt: {}", input.prompt);

    // Prepare mounts
    let group_dir = project_root.join("workspace");
    if !group_dir.exists() {
        fs::create_dir_all(&group_dir)?;
    }

    // Prepare OAuth config mounts (Detect multiple locations)
    let gemini_config_v1 = home_dir.join(".gemini");
    let gemini_config_v2 = home_dir.join(".config").join("gemini");
    
    let mut args = vec![
        "run".to_string(),
        "-i".to_string(),
        "--rm".to_string(),
        "-v".to_string(), format!("{}:/home/rclaw/workspace", group_dir.display()),
        "-w".to_string(), "/home/rclaw/workspace".to_string(),
        // Inyectar el ID del usuario actual para asegurar permisos y el HOME correcto
        "-u".to_string(), format!("{}:{}", unsafe { libc::getuid() }, unsafe { libc::getgid() }),
        "-e".to_string(), "HOME=/home/rclaw".to_string(),
    ];

    if gemini_config_v1.exists() {
        args.push("-v".to_string());
        args.push(format!("{}:/home/rclaw/.gemini", gemini_config_v1.display()));
    }
    
    if gemini_config_v2.exists() {
        args.push("-v".to_string());
        args.push(format!("{}:/home/rclaw/.config/gemini", gemini_config_v2.display()));
    }

    args.push("rclaw-agent:latest".to_string());

    debug!("Docker args: {:?}", args);

    let mut child = Command::new("docker")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn docker. Is it installed and is the image built?")?;

    // Send input via stdin
    if let Some(mut stdin) = child.stdin.take() {
        let input_json = serde_json::to_string(input)?;
        stdin.write_all(input_json.as_bytes())?;
    }

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
    info!("Container finished in {:?}", duration);

    // Filtrar ruidos
    let filtered_stderr = stderr
        .lines()
        .filter(|line| {
            !line.contains("DeprecationWarning")
                && !line.contains("punycode")
                && !line.contains("YOLO mode")
                && !line.contains("Loaded cached credentials")
        })
        .collect::<Vec<_>>()
        .join("\n");

    if !status.success() && !filtered_stderr.trim().is_empty() {
        error!("Container failed: {}", filtered_stderr);
        return Ok(ContainerOutput {
            status: "error".to_string(),
            result: None,
            new_session_id: None,
            error: Some(format!("Exit code {}: {}", status, filtered_stderr)),
        });
    }

    // Procesar el stream-json
    let mut combined_output: Vec<String> = Vec::new();

    for line in stdout.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            match val["type"].as_str() {
                Some("message") => {
                    if let Some(content) = val["content"].as_str() {
                        if val["role"].as_str() == Some("assistant") {
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
