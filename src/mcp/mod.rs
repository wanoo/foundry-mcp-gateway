//! Couche MCP maison : JSON-RPC 2.0 + sessions + notifications.
//! Implémentée directement sur la spec (2025-03-26) — pas de SDK, pas de dérive.

pub mod auth;
pub mod http;
pub mod prompts;
pub mod resources;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};

use crate::foundry::auth::Credential;
use crate::foundry::client::FoundryHandle;
use crate::tools;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Versions de spec acceptées (on renvoie celle demandée si connue).
pub const SUPPORTED_PROTOCOLS: [&str; 2] = ["2025-06-18", "2025-03-26"];
pub const SERVER_NAME: &str = "foundry-mcp-gateway";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Session {
    /// Canal des notifications serveur → client (flux SSE GET).
    pub notify: mpsc::Sender<Value>,
    /// Dernière activité, en millisecondes depuis la création de l'état.
    /// Un AtomicU64 plutôt qu'un Mutex : c'est touché à chaque requête.
    pub last_seen: std::sync::atomic::AtomicU64,
}

/// Une session sans activité ni flux SSE ouvert est abandonnée au-delà de ce
/// délai. Large exprès : un MJ qui laisse son client MCP ouvert entre deux
/// scènes ne doit jamais voir sa session coupée sous lui.
const SESSION_TTL_MS: u64 = 30 * 60_000;
/// Un récepteur SSE que personne ne vient chercher : le client a fait
/// `initialize` puis n'a jamais ouvert le flux (ou est mort en route).
const PENDING_RX_TTL_MS: u64 = 5 * 60_000;
/// Garde-fou de pointe : le TTL borne la fuite dans le temps, ce plafond la
/// borne en volume si quelqu'un boucle sur `initialize`.
pub const MAX_SESSIONS: usize = 1024;

/// Récepteurs de notifications garés entre `initialize` (POST) et l'ouverture
/// du flux (GET), avec l'instant où ils ont été garés — c'est lui qui permet
/// au balayage de récupérer ceux que personne n'est jamais venu chercher.
pub type PendingRx = Arc<Mutex<HashMap<String, (mpsc::Receiver<Value>, u64)>>>;

/// Horloge monotone du processus, en millisecondes.
pub fn now_ms() -> u64 {
    static START: std::sync::LazyLock<std::time::Instant> =
        std::sync::LazyLock::new(std::time::Instant::now);
    START.elapsed().as_millis() as u64
}

/// Réclame les sessions mortes.
///
/// Sans ça, une session ne disparaissait qu'avec un DELETE explicite : un
/// client qui se déconnecte brutalement — le cas courant — laissait derrière
/// lui un `Receiver(64)` et un `Arc<Session>` pour la vie du processus. Sur un
/// service qui tourne des semaines, la croissance est monotone.
///
/// Deux critères, parce qu'ils attrapent deux fuites différentes :
///   · `notify.is_closed()` — le client a fermé son flux SSE, la session est
///     morte tout de suite, sans attendre le TTL ;
///   · le TTL — le client n'a jamais ouvert de flux et n'a jamais dit au revoir.
///
/// ⚠️ Lancée depuis `main`, jamais depuis `router()` : `--dump-tools` construit
/// un état et ne doit pas se retrouver avec une tâche de fond.
pub fn spawn_session_reaper(state: McpState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let now = now_ms();
            let mut sessions = state.sessions.lock().await;
            let before = sessions.len();
            sessions.retain(|_, s| {
                !s.notify.is_closed()
                    && now.saturating_sub(s.last_seen.load(std::sync::atomic::Ordering::Relaxed))
                        < SESSION_TTL_MS
            });
            let dropped = before - sessions.len();
            let alive: std::collections::HashSet<String> = sessions.keys().cloned().collect();
            drop(sessions);

            let mut pending = state.pending_rx.lock().await;
            pending.retain(|sid, (_, born)| {
                alive.contains(sid) && now.saturating_sub(*born) < PENDING_RX_TTL_MS
            });
            drop(pending);

            if dropped > 0 {
                tracing::debug!(dropped, "sessions MCP réclamées");
            }
        }
    });
}

#[derive(Clone)]
pub struct McpState {
    pub foundry: FoundryHandle,
    pub sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
    /// URIs de ressources souscrites (partagées entre sessions, comme avant).
    pub subscriptions: Arc<Mutex<HashSet<String>>>,
    /// FOUNDRY_ADMIN_PASSWORD : requis par les outils admin_* (plan /setup).
    pub admin_password: Option<Arc<String>>,
    /// Dernier id de monde vu actif — cible par défaut d'admin_launch_world.
    pub last_world_id: Arc<Mutex<Option<String>>>,
    /// FOUNDRY_READONLY : n'expose et n'exécute que les outils en lecture seule.
    pub readonly: bool,
    /// Toutes les instances configurées (FOUNDRY_CREDENTIALS_JSON).
    pub credentials: Arc<Vec<Credential>>,
    /// Connexions vivantes, une par instance utilisée — ouvertes à la demande.
    pub pool: Arc<Mutex<HashMap<String, FoundryHandle>>>,
    /// Signe les commandes envoyées au compagnon (voir foundry::signing).
    pub signer: Arc<crate::foundry::signing::CommandSigner>,
    /// Récepteurs SSE en attente d'un GET, avec l'instant de création.
    /// Dans l'état et non dans un `static` : un global rendait le transport
    /// intestable et faisait fuiter les sessions entre deux instances.
    pub pending_rx: PendingRx,
    /// Instance visée par défaut quand un appel ne précise pas `instance`.
    pub active: Arc<Mutex<String>>,
}

impl McpState {
    pub fn new(foundry: FoundryHandle, credentials: Vec<Credential>) -> Self {
        let first = credentials
            .first()
            .map(|c| c.id.clone())
            .unwrap_or_default();
        let mut pool = HashMap::new();
        // La connexion déjà ouverte au démarrage sert la première instance.
        pool.insert(first.clone(), foundry.clone());
        Self {
            foundry,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            subscriptions: Arc::new(Mutex::new(HashSet::new())),
            admin_password: std::env::var("FOUNDRY_ADMIN_PASSWORD").ok().map(Arc::new),
            last_world_id: Arc::new(Mutex::new(None)),
            readonly: matches!(
                std::env::var("FOUNDRY_READONLY")
                    .unwrap_or_default()
                    .as_str(),
                "1" | "true" | "yes"
            ),
            credentials: Arc::new(credentials),
            pool: Arc::new(Mutex::new(pool)),
            active: Arc::new(Mutex::new(first)),
            // Une clé éphémère si l'env n'en fournit pas : la passerelle
            // démarre quand même (--dump-tools, essais), mais le compagnon
            // refusera ses commandes dangereuses tant que la clé publique
            // inscrite dans le monde ne correspond pas.
            signer: Arc::new(
                crate::foundry::signing::CommandSigner::from_env()
                    .map(|(s, _)| s)
                    .unwrap_or_else(|_| crate::foundry::signing::CommandSigner::generate()),
            ),
            pending_rx: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// L'état vu par une instance donnée — `None` = l'instance active.
    /// La connexion est ouverte à la demande puis gardée vivante : plusieurs
    /// mondes sont donc servis SIMULTANÉMENT, une socket chacun.
    pub async fn resolve(&self, instance: Option<&str>) -> anyhow::Result<McpState> {
        let id = match instance {
            Some(i) => i.to_string(),
            None => self.active.lock().await.clone(),
        };
        let handle = {
            let mut pool = self.pool.lock().await;
            match pool.get(&id) {
                Some(h) => h.clone(),
                None => {
                    let cred = self
                        .credentials
                        .iter()
                        .find(|c| c.id == id)
                        .ok_or_else(|| {
                            anyhow::anyhow!("instance inconnue : '{id}' (voir show_credentials)")
                        })?
                        .clone();
                    let h = crate::foundry::client::spawn(vec![cred]);
                    pool.insert(id.clone(), h.clone());
                    h
                }
            }
        };
        // Connexion fraîche : lui laisser le temps de s'établir, sinon le
        // premier appel échouerait bêtement sur « Not connected ».
        for _ in 0..40 {
            if handle.is_connected() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        Ok(McpState {
            foundry: handle,
            ..self.clone()
        })
    }

    pub async fn notify_all(&self, notification: Value) {
        let sessions = self.sessions.lock().await;
        for session in sessions.values() {
            let _ = session.notify.try_send(notification.clone());
        }
    }
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Traite un message JSON-RPC. `None` = notification (pas de réponse).
pub async fn handle_message(state: &McpState, message: &Value) -> Option<Value> {
    let method = message.get("method")?.as_str()?;
    let id = message.get("id").cloned();
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    // Notifications entrantes : rien à répondre.
    let id = id?;

    let result = match method {
        "initialize" => Ok(json!({
            // On parle la version demandée si on la connaît, la nôtre sinon.
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str)
                .filter(|v| SUPPORTED_PROTOCOLS.contains(v))
                .unwrap_or(PROTOCOL_VERSION),
            "capabilities": {
                "tools": {},
                "resources": { "subscribe": true },
                "prompts": {},
                "logging": {}
            },
            "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions(state) })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match tools::dispatch(state, name, &args).await {
                Ok(v) => Ok(v),
                Err(e) => return Some(rpc_error(&id, -32602, &format!("{e:#}"))),
            }
        }
        "resources/list" => {
            let cursor = params.get("cursor").and_then(Value::as_str);
            resources::resources_list(state, cursor)
                .await
                .map_err(|e| format!("{e:#}"))
        }
        "resources/read" => {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or_default();
            resources::resources_read(state, uri)
                .await
                .map_err(|e| format!("{e:#}"))
        }
        "resources/subscribe" => {
            if let Some(uri) = params.get("uri").and_then(Value::as_str) {
                state.subscriptions.lock().await.insert(uri.to_string());
            }
            Ok(json!({}))
        }
        "resources/unsubscribe" => {
            if let Some(uri) = params.get("uri").and_then(Value::as_str) {
                state.subscriptions.lock().await.remove(uri);
            }
            Ok(json!({}))
        }
        "prompts/list" => Ok(json!({ "prompts": prompts::prompt_definitions() })),
        "prompts/get" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            prompts::prompts_get(state, name, &args)
                .await
                .map_err(|e| format!("{e:#}"))
        }
        _ => Err(format!("méthode inconnue : {method}")),
    };

    Some(match result {
        Ok(value) => rpc_result(&id, value),
        Err(message) => rpc_error(&id, -32601, &message),
    })
}

/// Tâche de fond : relaie les broadcasts Foundry en notifications MCP
/// (logging allégé + resources/updated pour les URIs souscrites).
pub fn spawn_event_bridge(state: McpState) {
    let mut rx = state.foundry.subscribe_events();
    tokio::spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(e) => e,
                // Rafale : le pont a pris du retard sur le buffer de 256. On a
                // perdu n notifications, ce n'est pas une erreur de canal —
                // avant, ce cas faisait dormir le pont une seconde et le
                // réamorçait, c'est-à-dire qu'il décrochait précisément quand
                // il y avait du trafic (un import en masse, par exemple).
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "pont d'événements en retard");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    rx = state.foundry.subscribe_events();
                    continue;
                }
            };
            let first = event.args.first();
            let mut data = json!({ "seq": event.seq, "event": event.event });
            if event.event == "modifyDocument"
                && let Some(f) = first
            {
                data["type"] = f.get("type").cloned().unwrap_or(Value::Null);
                data["action"] = f.get("action").cloned().unwrap_or(Value::Null);
            }
            state
                .notify_all(json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/message",
                    "params": { "level": "info", "logger": "foundry-events", "data": data }
                }))
                .await;

            // resources/updated pour les documents souscrits
            if event.event == "modifyDocument"
                && let Some(f) = first
            {
                let section = match f.get("type").and_then(Value::as_str) {
                    Some("JournalEntry") => Some("journal"),
                    Some("Actor") => Some("actors"),
                    _ => None,
                };
                if let Some(section) = section {
                    let subs = state.subscriptions.lock().await.clone();
                    if !subs.is_empty() {
                        let ids = f
                            .get("result")
                            .and_then(Value::as_array)
                            .map(|docs| {
                                docs.iter()
                                    .filter_map(|d| {
                                        d.as_str().map(String::from).or_else(|| {
                                            d.get("_id").and_then(Value::as_str).map(String::from)
                                        })
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        for id in ids {
                            let uri = format!("foundry://{section}/{id}");
                            if subs.contains(&uri) {
                                state
                                    .notify_all(json!({
                                        "jsonrpc": "2.0",
                                        "method": "notifications/resources/updated",
                                        "params": { "uri": uri }
                                    }))
                                    .await;
                            }
                        }
                    }
                }
            }
        }
    });
}
