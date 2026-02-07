mod auth;
mod auth_discovery;
mod container;
mod db;
mod task_scheduler;
mod ui;

use crate::auth::setup_gemini_auth;
use crate::container::{run_container_agent, ContainerInput, RegisteredGroup};
use crate::db::Db;
use crate::task_scheduler::TaskScheduler;
use crate::ui::{run_tui, App, AppEvent, TuiLogger, WorkerEvent};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser)]
#[command(name = "rclaw")]
#[command(about = "Rust imitator of OpenClaw", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the bot loop with TUI
    Start,
    /// Run setup wizard
    Setup,
    /// Run a single agent execution (headless test)
    Run {
        #[arg(short, long)]
        prompt: String,
        #[arg(short, long, default_value = "main")]
        group: String,
    },
    /// Initialize or check DB
    DbCheck,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Configurar Logger: Si es modo Start (TUI), usamos el logger custom. Si no, stderr.
    let tui_logger = if let Some(Commands::Start) = &cli.command {
        Some(TuiLogger::new())
    } else {
        None
    };

    if let Some(logger) = &tui_logger {
        let subscriber = FmtSubscriber::builder()
            .with_max_level(Level::INFO)
            .with_writer(logger.clone())
            .without_time()
            .with_ansi(false)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    } else {
        // Modo headless estándar
        let subscriber = FmtSubscriber::builder()
            .with_max_level(Level::INFO)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    }

    let db_path = PathBuf::from("rclaw.db");

    match &cli.command {
        Some(Commands::Setup) => {
            info!("Starting setup wizard...");
            let db = Db::new(&db_path).expect("Failed to open DB");
            
            let has_auth = db.get_auth_key("gemini_access_token").unwrap_or(None).is_some();
            let has_image = std::process::Command::new("docker")
                .args(&["inspect", "--type=image", "rclaw-agent:latest"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            let mut run_auth = true;
            let mut run_build = true;

            if has_auth || has_image {
                println!("\n--- RClaw Setup Status ---");
                println!("Gemini Auth: {}", if has_auth { "CONFIGURED ✅" } else { "NOT FOUND ❌" });
                println!("Docker Image: {}", if has_image { "BUILT ✅" } else { "NOT FOUND ❌" });
                println!("--------------------------\n");

                if has_auth {
                    println!("What would you like to do?");
                    println!("1) Re-configure everything (New Auth + Rebuild Images)");
                    println!("2) Only rebuild Docker images (Keep current Auth)");
                    println!("3) Cancel");
                    
                    print!("\nSelect an option [1-3]: ");
                    use std::io::{self, Write};
                    io::stdout().flush().unwrap();

                    let mut choice = String::new();
                    io::stdin().read_line(&mut choice).unwrap();
                    match choice.trim() {
                        "1" => {
                            run_auth = true;
                            run_build = true;
                        }
                        "2" => {
                            run_auth = false;
                            run_build = true;
                        }
                        _ => {
                            println!("Setup cancelled.");
                            return;
                        }
                    }
                }
            }

            let mut auth_success = !run_auth;
            if run_auth {
                if let Some((access, refresh)) = setup_gemini_auth().await {
                    db.set_auth_key("gemini_access_token", &access).unwrap();
                    db.set_auth_key("gemini_refresh_token", &refresh).unwrap();
                    info!("Credentials saved to database.");
                    auth_success = true;
                } else {
                    error!("Gemini Auth failed.");
                }
            }

            if auth_success && run_build {
                info!("Stopping and removing existing rclaw-agent-singleton container...");
                let _ = std::process::Command::new("docker")
                    .args(&["stop", "rclaw-agent-singleton"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                let _ = std::process::Command::new("docker")
                    .args(&["rm", "rclaw-agent-singleton"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();

                info!("Building agent containers...");
                let status = std::process::Command::new("bash")
                    .arg("container/build.sh")
                    .status()
                    .expect("Failed to execute build script");

                if status.success() {
                    info!("Containers built successfully.");

                    // Sync initial memory to workspace preserving existing files
                    let memory_src_path = std::path::Path::new("container/setup/memory");

                    if memory_src_path.exists() {
                        std::fs::create_dir_all("workspace/memory").ok();

                        // Use rsync recursively (-a) and do not overwrite existing files (--ignore-existing)
                        let status = std::process::Command::new("rsync")
                            .args([
                                "-a",
                                "--ignore-existing",
                                "container/setup/memory/",
                                "workspace/memory/",
                            ])
                            .status();

                        match status {
                            Ok(s) if s.success() => info!("Initial memory synced to workspace."),
                            _ => error!("Failed to sync initial memory to workspace."),
                        }
                    }
                } else {
                    error!("Container build failed with exit code: {}", status);
                }
            }
        }
        Some(Commands::Start) => {
            // 0. Pre-flight check: Ensure docker image exists
            let check_image = std::process::Command::new("docker")
                .args(&["inspect", "--type=image", "rclaw-agent:latest"])
                .output();

            match check_image {
                Ok(output) if output.status.success() => {
                    // Solo procedemos si la imagen existe
                }
                _ => {
                    // Imprimir directamente a stderr para asegurar visibilidad antes de que el logger se apropie de todo
                    eprintln!("\n❌ ERROR CRÍTICO: No se ha encontrado la imagen de Docker 'rclaw-agent:latest'.");
                    eprintln!("👉 Por favor, ejecuta 'cargo run -- setup' primero para construir las imágenes necesarias.\n");
                    return;
                }
            }

            info!("Initializing Rclaw...");
            // Inicializar DB
            let db: Db = match Db::new(&db_path) {
                Ok(db) => db,
                Err(e) => {
                    error!("Failed to init DB: {}", e);
                    return;
                }
            };
            let db = Arc::new(db);

            info!("Database ready.");

            // Iniciar el planificador de tareas
            let task_scheduler = TaskScheduler::new(db.clone());
            tokio::spawn(async move {
                task_scheduler.run().await;
            });
            info!("Task scheduler initialized.");

            // Canales para comunicación TUI <-> Worker
            let (tx_app, rx_worker) = mpsc::channel();
            let (tx_worker, rx_app) = mpsc::channel();

            // Background worker para procesar inputs
            tokio::spawn(async move {
                info!("Worker thread started.");
                while let Ok(event) = rx_worker.recv() {
                    match event {
                        AppEvent::Input(prompt) => {
                            info!("Processing input: {}", prompt);

                            let group_config = RegisteredGroup {
                                name: "main".to_string(),
                                folder: "main".to_string(),
                            };

                            let input = ContainerInput {
                                prompt,
                                session_id: "interactive".to_string(),
                                group_folder: "main".to_string(),
                                chat_jid: "tui-user".to_string(),
                                is_main: true,
                                is_scheduled_task: None,
                            };

                            // Ejecutar agente en un hilo bloqueante pero sin mover el input permanentemente
                            let worker_tx = tx_worker.clone();
                            tokio::task::spawn_blocking(move || {
                                match run_container_agent(&group_config, &input) {
                                    Ok(output) => {
                                        if let Some(res) = output.result {
                                            let _ = worker_tx.send(WorkerEvent::Response(res));
                                        } else if let Some(err) = output.error {
                                            let _ = worker_tx.send(WorkerEvent::Response(format!(
                                                "Error: {}",
                                                err
                                            )));
                                        }
                                    }
                                    Err(e) => {
                                        let _ = worker_tx.send(WorkerEvent::Response(format!(
                                            "Container Error: {}",
                                            e
                                        )));
                                    }
                                }
                            });
                        }
                    }
                }
            });

            if let Some(logger) = tui_logger {
                let app = App::new(logger, tx_app, rx_app);
                if let Err(e) = run_tui(app) {
                    eprintln!("TUI Error: {}", e);
                }
            }
        }
        Some(Commands::Run { prompt, group }) => {
            info!(
                "Running agent for group '{}' with prompt: {}",
                group, prompt
            );

            let group_config = RegisteredGroup {
                name: group.clone(),
                folder: group.clone(),
            };

            let input = ContainerInput {
                prompt: prompt.clone(),
                session_id: "cli-run".to_string(),
                group_folder: group.clone(),
                chat_jid: "test-user@s.whatsapp.net".to_string(),
                is_main: group == "main",
                is_scheduled_task: None,
            };

            match tokio::task::spawn_blocking(move || run_container_agent(&group_config, &input))
                .await
            {
                Ok(Ok(output)) => {
                    info!("Agent finished: {:?}", output);
                }
                Ok(Err(e)) => {
                    info!("Agent failed: {:?}", e);
                }
                Err(e) => {
                    info!("Task join error: {:?}", e);
                }
            }
        }
        Some(Commands::DbCheck) => match Db::new(&db_path) {
            Ok(_) => info!("Database initialized successfully at {:?}", db_path),
            Err(e) => error!("Database init failed: {}", e),
        },
        None => {
            info!("No command specified. Use --help");
        }
    }
}
