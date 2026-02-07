use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Instant;
use tracing::{info, debug};
use std::fs;

#[derive(Debug, Serialize, Deserialize)]
pub struct ContainerInput {
    pub prompt: String,
    pub session_id: String,
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

fn wait_for_container_ready(container_name: &str) -> Result<()> {
    let start = Instant::now();
    let timeout = std::time::Duration::from_secs(10);

    debug!("Waiting for container {} to be ready...", container_name);

    while start.elapsed() < timeout {
        let output = Command::new("docker")
            .args(&["inspect", "-f", "{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Running}}{{end}}", container_name])
            .output()?;

        let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
        debug!("Current status for {}: {}", container_name, status);

        if status == "healthy" || status == "true" {
            debug!("Container {} is ready.", container_name);
            return Ok(());
        }

        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    anyhow::bail!("Timeout waiting for container {} to be ready", container_name);
}

pub fn run_container_agent(
    _group: &RegisteredGroup,
    input: &ContainerInput,
) -> Result<ContainerOutput> {
    let start_time = Instant::now();
    let project_root = std::env::current_dir().context("Failed to get current dir")?;
    let home_dir = dirs::home_dir().context("Failed to get home dir")?;
    let container_name = "rclaw-agent-singleton";

    info!("Ensuring rclaw-agent container is ready: {}", container_name);

    // Prepare mounts
    let group_dir = project_root.join("workspace");
    if !group_dir.exists() {
        fs::create_dir_all(&group_dir)?;
    }

    // 1. Check container existence and status
    let check_container = Command::new("docker")
        .args(&["inspect", "-f", "{{.State.Status}}", container_name])
        .output();

    let mut needs_wait = false;

    match check_container {
        Ok(output) if output.status.success() => {
            let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if status != "running" {
                info!("Starting existing container: {}", container_name);
                Command::new("docker")
                    .args(&["start", container_name])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()?;
                needs_wait = true;
            }
        }
        _ => {
            info!("Container {} not found. Creating...", container_name);
            
            let gemini_config_v1 = home_dir.join(".gemini");
            let gemini_config_v2 = home_dir.join(".config").join("gemini");
            
            let mut args = vec![
                "run".to_string(),
                "-d".to_string(),
                "--name".to_string(), container_name.to_string(),
                "-v".to_string(), format!("{}:/home/rclaw/workspace", group_dir.display()),
                "-w".to_string(), "/home/rclaw/workspace".to_string(),
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
            args.push("tail".to_string());
            args.push("-f".to_string());
            args.push("/dev/null".to_string());

            let status = Command::new("docker")
                .args(&args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;
            
            if !status.success() {
                return Ok(ContainerOutput {
                    status: "error".to_string(),
                    result: None,
                    new_session_id: None,
                    error: Some(format!("Failed to create container {}", container_name)),
                });
            }
            needs_wait = true;
        }
    }

    // 2. Wait for readiness
    if needs_wait {
        wait_for_container_ready(container_name)?;
    }

    // 3. Interaction via docker exec
    debug!("Executing prompt in container via docker exec");
    let mut child = Command::new("docker")
        .args(&["exec", "-i", container_name, "node", "/home/rclaw/entrypoint.js"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to execute docker exec")?;

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
    info!("Exec command finished in {:?}", duration);

    // Filtrar ruidos de stderr
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

    if !status.success() {
        return Ok(ContainerOutput {
            status: "error".to_string(),
            result: None,
            new_session_id: None,
            error: Some(format!("Container error (exit status: {}): {}", status, filtered_stderr)),
        });
    }

    // --- Procesamiento robusto del stream-json ---
    let mut final_result = String::new();
    let mut last_type = String::new();

    for line in stdout.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            let msg_type = val["type"].as_str().unwrap_or("");
            
            match msg_type {
                "message" => {
                    if let Some(content) = val["content"].as_str() {
                        if val["role"].as_str() == Some("assistant") {
                            // Separar bloques con UN espacio horizontal (doble \n)
                            if last_type == "tool_use" || last_type == "tool_result" {
                                if !final_result.is_empty() && !final_result.ends_with("\n\n") {
                                    final_result.push_str("\n\n");
                                }
                            }
                            final_result.push_str(content);
                            last_type = "message".to_string();
                        }
                    }
                }
                "tool_use" => {
                    let tool_name = val["tool_name"].as_str().unwrap_or("unknown");
                    let mut tool_desc = tool_name.to_string();
                    if let Some(params) = val["parameters"].as_object() {
                        if let Some(cmd) = params.get("command").and_then(|c| c.as_str()) {
                            tool_desc = format!("{} ({})", tool_name, cmd);
                        }
                    }
                    
                    if !final_result.is_empty() && !final_result.ends_with("\n\n") {
                        final_result.push_str("\n\n");
                    }
                    final_result.push_str(&format!("[RCLAW_USE_TOOL]{}", tool_desc));
                    last_type = "tool_use".to_string();
                }
                "tool_result" => {
                    if let Some(output) = val["output"].as_str() {
                        if !final_result.is_empty() && !final_result.ends_with("\n\n") {
                            final_result.push_str("\n\n");
                        }
                        // Usamos un marcador de fin explícito para que el parser no se trague el texto siguiente
                        final_result.push_str(&format!("[RCLAW_TOOL_RESULT]{}[RCLAW_END_RESULT]", output));
                        last_type = "tool_result".to_string();
                    }
                }
                _ => {}
            }
        }
    }

    Ok(ContainerOutput {
        status: "success".to_string(),
        result: Some(final_result.trim().to_string()),
        new_session_id: None,
        error: None,
    })
}
