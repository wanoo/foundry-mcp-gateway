//! foundry-mcp-rs — serveur MCP indépendant pour Foundry VTT.
//! Un seul binaire : client Foundry natif + transport MCP HTTP streamable.
//!
//! Env : MCP_ROOT_PRIVATE_KEY (jetons biscuit, chemin /mcp-<token>),
//! FOUNDRY_CREDENTIALS_JSON
//! (tableau [{_id, hostname, userid, password}] ; hostname peut inclure un
//! préfixe de route), PORT (défaut 8080).

mod foundry;
mod mcp;
mod systems;
mod tools;

use anyhow::{Context, Result};
use tracing::{info, warn};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
foundry-mcp-gateway — MCP server for Foundry VTT

USAGE:
    foundry-mcp                Run the server (see the environment below)
    foundry-mcp --dump-tools   Print every exposed tool as JSON, then exit
    foundry-mcp --dump-companion-commands  Print the client_* -> companion command map
    foundry-mcp --generate-key      Print a new companion signing key pair, then exit
    foundry-mcp --print-public-key  Print the public key of the configured signing key
    foundry-mcp --generate-mcp-key  Print a new MCP root key pair, then exit
    foundry-mcp --mint-token <label> [--readonly] [--days N]
                                    Mint a client token, then exit
    foundry-mcp --version      Print the version, then exit
    foundry-mcp --help         Print this help

ENVIRONMENT:
    MCP_ROOT_PRIVATE_KEY       Required (or MCP_ROOT_PUBLIC_KEY). Authenticates MCP
                               clients by biscuit token; the endpoint is /mcp-<token>.
                               Only needed in full if this gateway mints tokens
    MCP_REVOKED_TOKENS         Optional. Revocation ids to refuse, comma-separated
    FOUNDRY_CREDENTIALS_JSON   Required. [{_id, hostname, userid, password}, …]
    FOUNDRY_ADMIN_PASSWORD     Optional. Unlocks the admin_* tools
    FOUNDRY_READONLY           Optional. 1/true/yes: expose read-only tools only
    FOUNDRY_SYSTEMS            Optional. Restrict game-system modules
    COMPANION_SIGNING_KEY      Optional but recommended. Signs companion commands
                               (--generate-key). Without it a throwaway key is made
                               at every start, so the companion refuses risky commands
    FOUNDRY_ALLOW_INTERNAL_FETCH  Optional. 1/0 to force whether upload_file may reach
                               private addresses (auto-detected from your instances)
    PORT                       Optional. Defaults to 8080
";

/// `--dump-companion-commands` : la carte outil `client_*` → commande compagnon,
/// avec son mode de livraison. C'est le contrat que le `check-parity.sh` du
/// dépôt compagnon lit (via `docs/companion-commands.json`) pour vérifier
/// qu'aucune moitié n'a pris de l'avance sur l'autre. Aucune connexion requise.
fn dump_companion_commands() -> Result<()> {
    use tools::companion::{COMPANION_COMMANDS, Delivery};
    let rows: Vec<_> = COMPANION_COMMANDS
        .iter()
        .map(|(tool, cmd, delivery)| {
            let d = match delivery {
                Delivery::Scene => "scene",
                Delivery::Addressed => "addressed",
                Delivery::Unique => "unique",
            };
            serde_json::json!({ "tool": tool, "cmd": cmd, "delivery": d })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&rows)?);
    Ok(())
}

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

/// `--generate-key` : une paire de clés pour signer les commandes du compagnon.
///
/// La privée va dans l'environnement de la passerelle, la publique dans les
/// réglages du module côté monde. Aucune connexion nécessaire, pour qu'on
/// puisse préparer sa configuration avant même d'avoir un Foundry qui tourne.
fn generate_key() -> Result<()> {
    let signer = foundry::signing::CommandSigner::generate();
    println!("# Gateway side — keep this secret, put it in your environment:");
    println!("COMPANION_SIGNING_KEY={}", signer.private_hex());
    println!();
    println!("# World side — paste this into the companion module's settings");
    println!("# (Configure Settings → Foundry MCP Companion → Gateway public key),");
    println!("# or let the gateway install it for you with the companion_trust_key tool:");
    println!("{}", signer.public_hex());
    Ok(())
}

/// `--print-public-key` : la publique dérivée de la clé déjà configurée.
fn print_public_key() -> Result<()> {
    let (signer, from_env) = foundry::signing::CommandSigner::from_env()?;
    if !from_env {
        eprintln!(
            "warning: COMPANION_SIGNING_KEY is not set, so this key is ephemeral and \
             changes at every restart. Generate a stable one with --generate-key."
        );
    }
    println!("{}", signer.public_hex());
    Ok(())
}

/// `--generate-mcp-key` : la paire de clés racine qui authentifie les clients.
fn generate_mcp_key() -> Result<()> {
    let (private_hex, public_hex) = mcp::auth::generate_root();
    println!("# Gateway side — keep this secret:");
    println!("MCP_ROOT_PRIVATE_KEY={private_hex}");
    println!();
    println!("# A gateway that never mints tokens only needs the public half:");
    println!("MCP_ROOT_PUBLIC_KEY={public_hex}");
    println!();
    println!("# Then mint a client token:");
    println!("#   foundry-mcp --mint-token claude-code");
    println!("#   foundry-mcp --mint-token dashboard --readonly --days 90");
    Ok(())
}

/// `--mint-token <label> [--readonly] [--days N]` : un jeton pour un client.
fn mint_token(args: Vec<String>) -> Result<()> {
    let label = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "client".into());
    let scope = if args.iter().any(|a| a == "--readonly") {
        mcp::auth::Scope::ReadOnly
    } else {
        mcp::auth::Scope::Full
    };
    let days = args
        .iter()
        .position(|a| a == "--days")
        .and_then(|i| args.get(i + 1))
        .and_then(|d| d.parse().ok())
        .unwrap_or(mcp::auth::DEFAULT_TTL_DAYS);

    let private = std::env::var("MCP_ROOT_PRIVATE_KEY")
        .context("MCP_ROOT_PRIVATE_KEY is required to mint a token (see --generate-mcp-key)")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    let (token, revocation) = mcp::auth::mint_token(&private, &label, scope, days, now)?;

    println!(
        "# Token for \"{label}\" — {} access, valid {days} days.",
        if scope == mcp::auth::Scope::ReadOnly {
            "read-only"
        } else {
            "full"
        }
    );
    println!("# Treat the whole URL as a password: it is the credential.");
    println!("#   claude mcp add foundry --transport http https://<your-host>/mcp-{token}");
    println!();
    println!("{token}");
    println!();
    println!("# Revocation id — add it to MCP_REVOKED_TOKENS to kill this token alone:");
    println!("# {revocation}");
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
        Some("--dump-companion-commands") => return dump_companion_commands(),
        Some("--generate-key") => return generate_key(),
        Some("--print-public-key") => return print_public_key(),
        Some("--generate-mcp-key") => return generate_mcp_key(),
        Some("--mint-token") => return mint_token(std::env::args().skip(2).collect()),
        Some(other) => {
            eprintln!("unknown argument: {other}\n");
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

    // L'authentification des clients MCP : des jetons biscuit, vérifiés contre
    // la clé publique racine. Ils remplacent l'ancien MCP_SECRET, un secret
    // partagé sans portée, sans expiration et sans révocation possible.
    let auth = mcp::auth::McpAuth::from_env()?;
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
    mcp::spawn_session_reaper(state.clone());

    // La clé publique au démarrage : c'est la voie de secours pour la coller
    // à la main dans les réglages du module quand l'outil companion_trust_key
    // n'est pas praticable. Jamais la privée — les logs se conservent.
    if std::env::var("COMPANION_SIGNING_KEY").is_ok() {
        info!(
            "companion signing public key: {}",
            state.signer.public_hex()
        );
    } else {
        warn!(
            "COMPANION_SIGNING_KEY is not set: commands are signed with a throwaway key that \
             changes at every restart, so the companion will refuse risky commands. \
             Generate a stable key with --generate-key."
        );
    }

    let app = mcp::http::router(state, auth);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!("listening on {addr} · MCP endpoint /mcp-<token>");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
