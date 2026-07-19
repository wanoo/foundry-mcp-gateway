//! foundry-mcp-rs — serveur MCP indépendant pour Foundry VTT.
//! Un seul binaire : client Foundry natif + transport MCP HTTP streamable.
//!
//! Env : MCP_SECRET (chemin /mcp-<secret>), FOUNDRY_CREDENTIALS_JSON
//! (tableau [{_id, hostname, userid, password}] ; hostname peut inclure un
//! préfixe de route), PORT (défaut 8080).

mod foundry;
mod mcp;
mod systems;
mod tools;

use anyhow::{Context, Result};
use tracing::info;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
foundry-mcp-gateway — MCP server for Foundry VTT

USAGE:
    foundry-mcp                Run the server (see the environment below)
    foundry-mcp --dump-tools   Print every exposed tool as JSON, then exit
    foundry-mcp --version      Print the version, then exit
    foundry-mcp --help         Print this help

ENVIRONMENT:
    MCP_SECRET                 Required. Endpoint path becomes /mcp-<secret>
    FOUNDRY_CREDENTIALS_JSON   Required. [{_id, hostname, userid, password}, …]
    FOUNDRY_ADMIN_PASSWORD     Optional. Unlocks the admin_* tools
    FOUNDRY_READONLY           Optional. 1/true/yes: expose read-only tools only
    FOUNDRY_SYSTEMS            Optional. Restrict game-system modules
    PORT                       Optional. Defaults to 8080
";

/// `--dump-tools` : la liste des outils sans aucune connexion Foundry — sert de
/// source de vérité aux vérifications de doc en CI, et laisse un intégrateur
/// inspecter l'API sans rien déployer.
fn dump_tools() -> Result<()> {
    let handle = foundry::client::spawn(vec![]);
    let state = mcp::McpState::new(handle, vec![]);
    let tools = tools::definitions(&state);
    println!("{}", serde_json::to_string_pretty(&tools)?);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("foundry-mcp-gateway {VERSION}");
            return Ok(());
        }
        Some("--help" | "-h") => {
            print!("{USAGE}");
            return Ok(());
        }
        Some("--dump-tools") => return dump_tools(),
        Some(other) => {
            eprintln!("argument inconnu : {other}\n");
            print!("{USAGE}");
            std::process::exit(2);
        }
        None => {}
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let secret = std::env::var("MCP_SECRET").context("MCP_SECRET est requis")?;
    let creds_json =
        std::env::var("FOUNDRY_CREDENTIALS_JSON").context("FOUNDRY_CREDENTIALS_JSON est requis")?;
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let credentials = foundry::auth::parse_credentials(&creds_json)?;
    let handle = foundry::client::spawn(credentials.clone());
    let state = mcp::McpState::new(handle, credentials);
    mcp::spawn_event_bridge(state.clone());

    let app = mcp::http::router(state, &secret);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!("en écoute sur {addr} · MCP /mcp-<secret>");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
