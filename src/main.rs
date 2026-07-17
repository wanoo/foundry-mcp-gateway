//! foundry-mcp-rs — serveur MCP indépendant pour Foundry VTT.
//! Un seul binaire : client Foundry natif + transport MCP HTTP streamable.
//!
//! Env : MCP_SECRET (chemin /mcp-<secret>), FOUNDRY_CREDENTIALS_JSON
//! (tableau [{_id, hostname, userid, password}] ; hostname peut inclure un
//! préfixe de route), PORT (défaut 8080).

mod foundry;
mod mcp;
mod tools;

use anyhow::{Context, Result};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let secret = std::env::var("MCP_SECRET").context("MCP_SECRET est requis")?;
    let creds_json = std::env::var("FOUNDRY_CREDENTIALS_JSON")
        .context("FOUNDRY_CREDENTIALS_JSON est requis")?;
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);

    let credentials = foundry::auth::parse_credentials(&creds_json)?;
    let handle = foundry::client::spawn(credentials);
    let state = mcp::McpState::new(handle);
    mcp::spawn_event_bridge(state.clone());

    let app = mcp::http::router(state, &secret);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!("en écoute sur {addr} · MCP /mcp-<secret>");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
