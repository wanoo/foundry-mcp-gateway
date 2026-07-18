//! Couche MCP maison : JSON-RPC 2.0 + sessions + notifications.
//! Implémentée directement sur la spec (2025-03-26) — pas de SDK, pas de dérive.

pub mod http;

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
            tools::resources_list(state, cursor)
                .await
                .map_err(|e| format!("{e:#}"))
        }
        "resources/read" => {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or_default();
            tools::resources_read(state, uri)
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
        "prompts/list" => Ok(json!({ "prompts": tools::prompt_definitions() })),
        "prompts/get" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            tools::prompts_get(state, name, &args)
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
            let Ok(event) = rx.recv().await else {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                rx = state.foundry.subscribe_events();
                continue;
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
