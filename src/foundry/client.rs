//! Client Foundry natif : se loggue comme un joueur, parle socket.io sur WS,
//! corrèle les acks, bufferise les broadcasts, se reconnecte avec backoff.
//! Modèle acteur : une tâche possède la connexion ; l'API passe par un handle.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tracing::{info, warn};

use super::auth::{self, Credential};
use super::documents::{
    can_use_index, collection_to_type, filter_fields, matches_where, pushdown_query,
};
use super::protocol::{Frame, PONG, SOCKET_CONNECT, build_emit, parse_frame};

#[derive(Debug, Clone)]
pub struct BufferedEvent {
    pub seq: u64,
    pub event: String,
    pub args: Vec<Value>,
}

const EVENT_BUFFER_MAX: usize = 300;
const EVENT_ARGS_MAX_CHARS: usize = 60_000;
const IGNORED_EVENTS: [&str; 3] = ["userActivity", "getUserActivity", "time"];

struct Shared {
    connected: AtomicBool,
    ack_seq: AtomicU64,
    event_seq: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Vec<Value>>>>,
    events: Mutex<Vec<BufferedEvent>>,
    outgoing: mpsc::Sender<String>,
    event_tx: broadcast::Sender<BufferedEvent>,
    generation: Mutex<Option<u32>>,
    user_id: Mutex<Option<String>>,
    session_id: Mutex<Option<String>>,
    credentials: Vec<Credential>,
    active_index: std::sync::atomic::AtomicUsize,
}

/// Sentinelle interne : force la fermeture de la connexion (changement
/// d'instance) — jamais envoyée sur le réseau.
const RECONNECT_SENTINEL: &str = "\u{1}RECONNECT";

#[derive(Clone)]
pub struct FoundryHandle {
    shared: Arc<Shared>,
    pub http: reqwest::Client,
}

impl FoundryHandle {
    pub fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::SeqCst)
    }
    pub fn hostname(&self) -> String {
        let i = self.shared.active_index.load(Ordering::SeqCst);
        self.shared
            .credentials
            .get(i)
            .map(|c| c.hostname.clone())
            .unwrap_or_default()
    }
    pub async fn session_id(&self) -> Option<String> {
        self.shared.session_id.lock().await.clone()
    }
    /// Credentials configurés (sans mot de passe) + index actif.
    pub fn credentials_info(&self) -> (usize, Vec<Value>) {
        let active = self.shared.active_index.load(Ordering::SeqCst);
        let list = self
            .shared
            .credentials
            .iter()
            .enumerate()
            .map(|(i, c)| {
                json!({
                    "item_order": i,
                    "_id": c.id,
                    "hostname": c.hostname,
                    "userid": c.userid,
                    "active": i == active,
                })
            })
            .collect();
        (active, list)
    }
    pub async fn user_id(&self) -> Option<String> {
        self.shared.user_id.lock().await.clone()
    }
    pub async fn generation(&self) -> Option<u32> {
        *self.shared.generation.lock().await
    }
    pub fn event_seq(&self) -> u64 {
        self.shared.event_seq.load(Ordering::SeqCst)
    }
    pub fn subscribe_events(&self) -> broadcast::Receiver<BufferedEvent> {
        self.shared.event_tx.subscribe()
    }

    pub async fn get_events(
        &self,
        since: u64,
        event: Option<&str>,
        limit: Option<usize>,
    ) -> (u64, Vec<BufferedEvent>) {
        let buf = self.shared.events.lock().await;
        let mut out: Vec<BufferedEvent> = buf
            .iter()
            .filter(|e| e.seq > since && event.is_none_or(|ev| e.event == ev))
            .cloned()
            .collect();
        if let Some(l) = limit
            && out.len() > l
        {
            out = out.split_off(out.len() - l);
        }
        (self.shared.event_seq.load(Ordering::SeqCst), out)
    }

    /// Émission fire-and-forget.
    pub async fn emit(&self, event: &str, args: &[Value]) -> Result<()> {
        if !self.is_connected() {
            bail!("Not connected to Foundry server");
        }
        self.shared
            .outgoing
            .send(build_emit(event, args, None))
            .await
            .map_err(|_| anyhow!("connexion socket fermée"))
    }

    /// Émission avec ack (timeout 30 s).
    pub async fn emit_with_ack(&self, event: &str, args: &[Value]) -> Result<Vec<Value>> {
        if !self.is_connected() {
            bail!("Not connected to Foundry server");
        }
        let id = self.shared.ack_seq.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.shared.pending.lock().await.insert(id, tx);
        self.shared
            .outgoing
            .send(build_emit(event, args, Some(id)))
            .await
            .map_err(|_| anyhow!("connexion socket fermée"))?;
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(payload)) => Ok(payload),
            Ok(Err(_)) => bail!("ack {event} abandonné (reconnexion ?)"),
            Err(_) => {
                self.shared.pending.lock().await.remove(&id);
                bail!("timeout (30 s) en attendant l'ack de {event}")
            }
        }
    }

    /// « modifyDocument » — le canal universel (get/create/update/delete).
    pub async fn modify_document(
        &self,
        doc_type: &str,
        action: &str,
        operation: Value,
    ) -> Result<Value> {
        let payload = json!({ "type": doc_type, "action": action, "operation": operation });
        let mut reply = self.emit_with_ack("modifyDocument", &[payload]).await?;
        let data = if reply.is_empty() {
            Value::Null
        } else {
            reply.remove(0)
        };
        if let Some(err) = data.get("error")
            && !err.is_null()
        {
            bail!("erreur Foundry ({doc_type} {action}) : {err}");
        }
        Ok(data)
    }

    /// Lecture d'une collection monde (ou d'un pack) par socket get.
    pub async fn get_collection(
        &self,
        doc_type: &str,
        query: Map<String, Value>,
        index: bool,
        pack: Option<&str>,
    ) -> Result<Vec<Value>> {
        let mut op = json!({ "query": query, "action": "get", "broadcast": false, "index": index });
        if let Some(p) = pack {
            op["pack"] = json!(p);
        }
        let data = self.modify_document(doc_type, "get", op).await?;
        Ok(data
            .get("result")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Le dump complet du monde (métadonnées + packs) — à réserver à get_world.
    pub async fn request_world(&self) -> Result<Value> {
        let mut payload = self.emit_with_ack("world", &[]).await?;
        if payload.is_empty() {
            bail!("réponse world vide");
        }
        Ok(payload.remove(0))
    }

    /// Lecture haut niveau : pushdown + index auto + filtre where complet +
    /// projection — avec repli en documents complets si l'index est inutilisable.
    pub async fn get_documents(
        &self,
        collection: &str,
        where_: Option<&Map<String, Value>>,
        fields: Option<&[String]>,
        offset: usize,
        limit: Option<usize>,
    ) -> Result<Vec<Value>> {
        let doc_type = collection_to_type(collection)
            .ok_or_else(|| anyhow!("collection inconnue : {collection}"))?;
        let query = pushdown_query(where_);
        let use_index = can_use_index(fields, where_);
        let mut docs = self
            .get_collection(doc_type, query.clone(), use_index, None)
            .await?;
        if use_index && !docs.is_empty() && docs.iter().any(|d| d.get("_id").is_none()) {
            warn!("index inutilisable pour {collection}, repli en documents complets");
            docs = self.get_collection(doc_type, query, false, None).await?;
        }
        let filtered = docs
            .into_iter()
            .filter(|d| where_.is_none_or(|w| matches_where(d, w)))
            .map(|d| filter_fields(&d, fields));
        let out: Vec<Value> = match limit {
            Some(l) => filtered.skip(offset).take(l).collect(),
            None => filtered.skip(offset).collect(),
        };
        Ok(out)
    }

    /// Un document par _id ou nom (pushdown ciblé).
    pub async fn get_document(
        &self,
        collection: &str,
        id: Option<&str>,
        name: Option<&str>,
        fields: Option<&[String]>,
    ) -> Result<Option<Value>> {
        let mut where_ = Map::new();
        match (id, name) {
            (Some(i), _) => {
                where_.insert("_id".into(), json!(i));
            }
            (None, Some(n)) => {
                where_.insert("name".into(), json!(n));
            }
            _ => bail!("id ou name requis"),
        }
        let docs = self
            .get_documents(collection, Some(&where_), fields, 0, Some(1))
            .await?;
        Ok(docs.into_iter().next())
    }

    /// Résolution souple : essaie par _id puis par nom.
    pub async fn find_document(
        &self,
        collection: &str,
        id_or_name: &str,
        fields: Option<&[String]>,
    ) -> Result<Option<Value>> {
        if let Some(d) = self
            .get_document(collection, Some(id_or_name), None, fields)
            .await?
        {
            return Ok(Some(d));
        }
        self.get_document(collection, None, Some(id_or_name), fields)
            .await
    }

    /// « manageFiles » (browse, createDirectory) — mêmes payloads que FilePicker.
    pub async fn manage_files(&self, data: Value, options: Value) -> Result<Value> {
        let mut reply = self.emit_with_ack("manageFiles", &[data, options]).await?;
        let result = if reply.is_empty() {
            Value::Null
        } else {
            reply.remove(0)
        };
        if let Some(err) = result.get("error")
            && !err.is_null()
        {
            bail!("manageFiles : {err}");
        }
        Ok(result)
    }

    /// « manageCompendium » (create / delete de packs).
    pub async fn manage_compendium(&self, action: &str, data: Value) -> Result<Value> {
        let payload = json!({ "action": action, "data": data, "options": {} });
        let mut reply = self.emit_with_ack("manageCompendium", &[payload]).await?;
        let result = if reply.is_empty() {
            Value::Null
        } else {
            reply.remove(0)
        };
        if let Some(err) = result.get("error")
            && !err.is_null()
        {
            bail!("manageCompendium {action} : {err}");
        }
        Ok(result)
    }

    /// Upload HTTP multipart vers /upload (cookie de session requis).
    pub async fn upload_file(
        &self,
        target: &str,
        filename: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<Value> {
        let session = self
            .session_id()
            .await
            .ok_or_else(|| anyhow!("Not connected to Foundry server"))?;
        let hostname = self.hostname();
        let (host, base) = super::auth::split_host(&hostname);
        let url = format!("https://{host}{base}/upload");
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str(content_type)?;
        let form = reqwest::multipart::Form::new()
            .text("source", "data")
            .text("target", target.to_string())
            .part("upload", part);
        let resp = self
            .http
            .post(&url)
            .header(reqwest::header::COOKIE, format!("session={session}"))
            .multipart(form)
            .send()
            .await?;
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if let Some(err) = body.get("error")
            && !err.is_null()
        {
            bail!("upload : {err}");
        }
        Ok(body)
    }

    /// Attend un événement matchant `predicate` (scan du buffer depuis
    /// `since_seq`, puis flux live), avec timeout. `None` = délai dépassé.
    pub async fn wait_for_event<F>(
        &self,
        since_seq: u64,
        timeout: std::time::Duration,
        predicate: F,
    ) -> Option<BufferedEvent>
    where
        F: Fn(&BufferedEvent) -> bool,
    {
        {
            let buf = self.shared.events.lock().await;
            if let Some(e) = buf.iter().find(|e| e.seq > since_seq && predicate(e)) {
                return Some(e.clone());
            }
        }
        let mut rx = self.subscribe_events();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(e)) => {
                    if e.seq > since_seq && predicate(&e) {
                        return Some(e);
                    }
                }
                Ok(Err(_)) => rx = self.subscribe_events(),
                Err(_) => return None,
            }
        }
    }
}

/// Lance le client : rend le handle immédiatement, la connexion (et la
/// reconnexion infinie à backoff) vit dans une tâche de fond.
pub fn spawn(credentials: Vec<Credential>) -> FoundryHandle {
    let (out_tx, out_rx) = mpsc::channel::<String>(256);
    let (event_tx, _) = broadcast::channel(256);
    let shared = Arc::new(Shared {
        connected: AtomicBool::new(false),
        ack_seq: AtomicU64::new(1),
        event_seq: AtomicU64::new(0),
        pending: Mutex::new(HashMap::new()),
        events: Mutex::new(Vec::new()),
        outgoing: out_tx,
        event_tx,
        generation: Mutex::new(None),
        user_id: Mutex::new(None),
        session_id: Mutex::new(None),
        credentials,
        active_index: std::sync::atomic::AtomicUsize::new(0),
    });
    let http = reqwest::Client::builder()
        .user_agent("foundry-mcp-gateway")
        .build()
        .expect("client http");
    let handle = FoundryHandle {
        shared: shared.clone(),
        http: http.clone(),
    };
    tokio::spawn(run_loop(shared, http, out_rx));
    handle
}

async fn run_loop(shared: Arc<Shared>, http: reqwest::Client, mut out_rx: mpsc::Receiver<String>) {
    let mut delay = std::time::Duration::from_secs(5);
    loop {
        match connect_once(&shared, &http, &mut out_rx).await {
            Ok(()) => {
                // connexion terminée proprement (coupure) : repartir vite
                delay = std::time::Duration::from_secs(5);
            }
            Err(e) => {
                warn!("connexion Foundry échouée : {e:#} — nouvel essai dans {delay:?}");
            }
        }
        shared.connected.store(false, Ordering::SeqCst);
        // abandonner les acks en attente (les requêtes échouent explicitement)
        shared.pending.lock().await.clear();
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(std::time::Duration::from_secs(60));
    }
}

async fn connect_once(
    shared: &Arc<Shared>,
    http: &reqwest::Client,
    out_rx: &mut mpsc::Receiver<String>,
) -> Result<()> {
    let index = shared.active_index.load(Ordering::SeqCst);
    let cred = shared
        .credentials
        .get(index)
        .context("aucun credential configuré")?;
    let generation = auth::detect_generation(http, &cred.hostname).await;
    *shared.generation.lock().await = generation;
    info!(hostname = %cred.hostname, ?generation, "connexion Foundry…");

    let session = auth::get_session(http, &cred.hostname).await?;
    auth::authenticate(http, &cred.hostname, &session, cred).await?;
    *shared.session_id.lock().await = Some(session.clone());

    let (url, cookie) = auth::socket_url_and_cookie(&cred.hostname, &session, generation);
    let mut request = url.clone().into_client_request()?;
    if let Some(c) = cookie {
        request.headers_mut().insert(
            tokio_tungstenite::tungstenite::http::header::COOKIE,
            c.parse().unwrap(),
        );
    }
    // Le dump du monde pèse des dizaines de Mo (Foundry annonce maxPayload
    // 100 Mo dans son handshake) : relever les limites par défaut (16 Mio).
    let ws_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(128 * 1024 * 1024))
        .max_frame_size(Some(128 * 1024 * 1024));
    let (ws, _) = tokio_tungstenite::connect_async_with_config(request, Some(ws_config), false)
        .await
        .context("connexion WebSocket")?;
    let (mut sink, mut stream) = ws.split();
    info!("WebSocket établi, attente du handshake…");

    loop {
        tokio::select! {
            Some(frame) = out_rx.recv() => {
                if frame == RECONNECT_SENTINEL {
                    bail!("reconnexion demandée (changement d'instance)");
                }
                sink.send(Message::Text(frame.into())).await.context("envoi WS")?;
            }
            msg = stream.next() => {
                let Some(msg) = msg else { bail!("WebSocket fermé par le serveur") };
                let msg = msg.context("lecture WS")?;
                let Message::Text(text) = msg else { continue };
                match parse_frame(&text) {
                    Frame::Ping => {
                        sink.send(Message::Text(PONG.into())).await.ok();
                    }
                    Frame::Handshake(_) => {
                        sink.send(Message::Text(SOCKET_CONNECT.into())).await.ok();
                    }
                    Frame::SocketConnected(_) => {}
                    Frame::Event(event, args) => {
                        if event == "session" {
                            let user = args.first()
                                .and_then(|a| a.get("userId"))
                                .and_then(Value::as_str);
                            match user {
                                Some(uid) => {
                                    *shared.user_id.lock().await = Some(uid.to_string());
                                    shared.connected.store(true, Ordering::SeqCst);
                                    info!(user = uid, "session Foundry prête");
                                }
                                None => bail!("session non liée (binding refusé)"),
                            }
                        } else {
                            record_event(shared, event, args).await;
                        }
                    }
                    Frame::Ack(id, payload) => {
                        if let Some(tx) = shared.pending.lock().await.remove(&id) {
                            let _ = tx.send(payload);
                        }
                    }
                    Frame::Other(_) => {}
                }
            }
        }
    }
}

async fn record_event(shared: &Arc<Shared>, event: String, mut args: Vec<Value>) {
    if IGNORED_EVENTS.contains(&event.as_str()) {
        return;
    }
    // Le canal du compagnon porte les RÉPONSES aux outils client_* (dont les
    // captures d'écran) : les tronquer les détruirait.
    let raw = serde_json::to_string(&args).unwrap_or_default();
    if !event.starts_with("module.") && raw.len() > EVENT_ARGS_MAX_CHARS {
        args = vec![json!({"truncated": true, "bytes": raw.len()})];
    }
    let seq = shared.event_seq.fetch_add(1, Ordering::SeqCst) + 1;
    let entry = BufferedEvent { seq, event, args };
    let mut buf = shared.events.lock().await;
    buf.push(entry.clone());
    if buf.len() > EVENT_BUFFER_MAX {
        buf.remove(0);
    }
    drop(buf);
    let _ = shared.event_tx.send(entry);
}
