use std::path::{Path, PathBuf};
use std::fs;
use regex::Regex;
use tracing::{info, debug};
use std::process::Command;

#[derive(Debug)]
pub struct DiscoveredCredentials {
    pub client_id: String,
    pub client_secret: String,
}

pub fn try_discover_gemini_credentials() -> Option<DiscoveredCredentials> {
    info!("Attempting to auto-discover Gemini CLI credentials...");

    // Estrategia 1: Buscar 'gemini' en el PATH y seguir el rastro
    if let Some(path) = find_in_path("gemini") {
        debug!("Found gemini binary at {:?}", path);
        if let Ok(real_path) = fs::canonicalize(&path) {
            debug!("Resolved gemini real path to {:?}", real_path);
            if let Some(parent) = real_path.parent().and_then(|p| p.parent()) {
                // Estamos en el directorio base del paquete (ej: /usr/local o la carpeta de brew)
                if let Some(creds) = find_oauth2_js_recursive(parent, 10) {
                    return Some(creds);
                }
            }
        }
    }

    // Estrategia 2: Buscar en rutas globales de npm comunes (como fallback)
    let search_paths = vec![
        dirs::home_dir().map(|h| h.join(".npm-global/lib/node_modules")),
        Some(PathBuf::from("/usr/local/lib/node_modules")),
        Some(PathBuf::from("/usr/lib/node_modules")),
        // Si el usuario usa nvm
        dirs::home_dir().map(|h| h.join(".nvm/versions/node")), 
    ];

    for base_path in search_paths.into_iter().flatten() {
        if !base_path.exists() { continue; }
        
        // Si es directorio nvm, iteramos versiones
        if base_path.to_string_lossy().contains(".nvm") {
             if let Ok(entries) = fs::read_dir(&base_path) {
                for entry in entries.flatten() {
                    let lib_path = entry.path().join("lib/node_modules");
                    if let Some(creds) = check_node_modules(&lib_path) {
                        return Some(creds);
                    }
                }
             }
        } else {
            if let Some(creds) = check_node_modules(&base_path) {
                return Some(creds);
            }
        }
    }

    // Estrategia 3: Intentar preguntar a npm root -g (si npm está en path)
    if let Ok(output) = Command::new("npm").args(["root", "-g"]).output() {
        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path_str.is_empty() {
            if let Some(creds) = check_node_modules(&PathBuf::from(path_str)) {
                return Some(creds);
            }
        }
    }

    None
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_env) {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn find_oauth2_js_recursive(dir: &Path, depth: usize) -> Option<DiscoveredCredentials> {
    if depth == 0 { return None; }

    if let Ok(entries) = fs::read_dir(dir) {
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && p.file_name() == Some(std::ffi::OsStr::new("oauth2.js")) {
                if let Ok(content) = fs::read_to_string(&p) {
                    if let Some(creds) = extract_credentials_from_file(&content) {
                        debug!("Found oauth2.js at {:?}", p);
                        return Some(creds);
                    }
                }
            } else if p.is_dir() {
                let name = p.file_name()?.to_string_lossy();
                if !name.starts_with('.') {
                    subdirs.push(p);
                }
            }
        }
        
        // Recurse subdirs
        for subdir in subdirs {
            if let Some(creds) = find_oauth2_js_recursive(&subdir, depth - 1) {
                return Some(creds);
            }
        }
    }
    None
}

fn check_node_modules(node_modules: &Path) -> Option<DiscoveredCredentials> {
    let gemini_core_path = node_modules.join("@google/gemini-cli-core");
    
    // Rutas posibles del archivo oauth2.js dentro del paquete (basado en la lógica de OpenClaw)
    let candidates = vec![
        gemini_core_path.join("dist/src/code_assist/oauth2.js"),
        gemini_core_path.join("dist/code_assist/oauth2.js"),
    ];

    for path in candidates {
        if path.exists() {
            debug!("Found potential oauth2.js at {:?}", path);
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(creds) = extract_credentials_from_file(&content) {
                    info!("Successfully extracted credentials from Gemini CLI installation!");
                    return Some(creds);
                }
            }
        }
    }
    None
}

fn extract_credentials_from_file(content: &str) -> Option<DiscoveredCredentials> {
    // Regex adaptadas de OpenClaw
    // idMatch: /(\d+-[a-z0-9]+\.apps\.googleusercontent\.com)/
    // secretMatch: /(GOCSPX-[A-Za-z0-9_-]+)/
    
    let re_id = Regex::new(r"(\d+-[a-z0-9]+\.apps\.googleusercontent\.com)").ok()?;
    let re_secret = Regex::new(r"(GOCSPX-[A-Za-z0-9_-]+)").ok()?;

    let client_id = re_id.find(content)?.as_str().to_string();
    let client_secret = re_secret.find(content)?.as_str().to_string();

    Some(DiscoveredCredentials {
        client_id,
        client_secret,
    })
}
