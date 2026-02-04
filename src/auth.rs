use std::io::{self, Write};
use crate::auth_discovery::try_discover_gemini_credentials;
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge, RedirectUrl, Scope,
    TokenUrl,
};
use tracing::{error, warn};
use url::Url;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REDIRECT_URI: &str = "http://localhost:8085/oauth2callback";

pub async fn setup_gemini_auth() -> Option<(String, String)> {
    println!("\n🦐 Rclaw Setup: Google Gemini CLI\n");

    let mut client_id = String::new();
    let mut client_secret = String::new();

    // 1. Intentar autodescubrimiento
    if let Some(creds) = try_discover_gemini_credentials() {
        println!("✅ Detected installed Gemini CLI credentials.");
        println!("   Using Client ID: {}...", &creds.client_id[..10]);
        client_id = creds.client_id;
        client_secret = creds.client_secret;
    } else {
        println!("⚠️  Could not auto-discover Gemini CLI credentials.");
        println!("   Please enter your Google Cloud OAuth credentials.");
        
        print!("   Client ID: ");
        io::stdout().flush().unwrap();
        io::stdin().read_line(&mut client_id).unwrap();
        client_id = client_id.trim().to_string();

        print!("   Client Secret: ");
        io::stdout().flush().unwrap();
        io::stdin().read_line(&mut client_secret).unwrap();
        client_secret = client_secret.trim().to_string();
    }

    if client_id.is_empty() || client_secret.is_empty() {
        error!("Missing credentials. Setup aborted.");
        return None;
    }

    // 2. Configurar cliente OAuth
    let client = BasicClient::new(ClientId::new(client_id.clone()))
        .set_client_secret(ClientSecret::new(client_secret.clone()))
        .set_auth_uri(AuthUrl::new(AUTH_URL.to_string()).unwrap())
        .set_token_uri(TokenUrl::new(TOKEN_URL.to_string()).unwrap())
        .set_redirect_uri(RedirectUrl::new(REDIRECT_URI.to_string()).unwrap());

    // 3. Generar URL (PKCE)
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, _csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("https://www.googleapis.com/auth/cloud-platform".to_string()))
        .add_scope(Scope::new("https://www.googleapis.com/auth/userinfo.email".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    println!("\n👉 Open this URL in your LOCAL browser to authorize:\n");
    println!("{}\n", auth_url);
    println!("👉 After authorizing, Google will redirect you to localhost.");
    println!("👉 Copy that FULL localhost URL and paste it here:\n");

    print!("> ");
    io::stdout().flush().unwrap();
    let mut redirect_input = String::new();
    io::stdin().read_line(&mut redirect_input).unwrap();
    
    let redirect_input = redirect_input.trim();
    
    // Parsear el código de la URL pegada
    let code = if let Ok(url) = Url::parse(redirect_input) {
        if let Some((_, code)) = url.query_pairs().find(|(k, _)| k == "code") {
            code.into_owned()
        } else {
            error!("URL does not contain 'code' parameter.");
            return None;
        }
    } else {
        // Asumir que el usuario pegó solo el código si no es URL válida
        warn!("Input is not a URL, assuming raw code.");
        redirect_input.to_string()
    };

    println!("\n🔄 Exchanging code for tokens...");

    // 4. Canjear código
    // (Nota: oauth2 crate reqwest feature is async)
    use oauth2::{AuthorizationCode, TokenResponse};
    
    // Explicit type annotation for result
    let http_client = reqwest::Client::new();
    let token_result = client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await;

    match token_result {
        Ok(token) => {
            let access = token.access_token().secret().clone();
            let refresh = token.refresh_token().map(|t| t.secret().clone()).unwrap_or_default();
            println!("✅ Authentication successful!");
            Some((access, refresh))
        },
        Err(e) => {
            error!("Token exchange failed: {:?}", e);
            None
        }
    }
}
