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
    events: Mutex<std::collections::VecDeque<BufferedEvent>>,
    outgoing: mpsc::Sender<String>,
    event_tx: broadcast::Sender<BufferedEvent>,
    generation: Mutex<Option<u32>>,
    /// Id du monde servi (`server.world` de /api/status), mis en cache au
    /// premier besoin. Sert à lier une commande signée au monde qui la reçoit :
    /// le compagnon le compare à son propre `game.world.id`.
    world_id: Mutex<Option<String>>,
    user_id: Mutex<Option<String>>,
    session_id: Mutex<Option<String>>,
    credentials: Vec<Credential>,
    active_index: std::sync::atomic::AtomicUsize,
}

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

    /// Id du monde servi, mis en cache après le premier appel.
    ///
    /// Lu depuis `server.world` de /api/status — la seule source légère : le
    /// dump `request_world()` pèse ~18 Mo, impensable à chaque commande. Le
    /// compagnon compare cette valeur à son propre `game.world.id`, ce qui
    /// empêche de rejouer dans un monde une commande signée pour un autre.
    pub async fn world_id(&self) -> Option<String> {
        if let Some(w) = self.shared.world_id.lock().await.clone() {
            return Some(w);
        }
        let hostname = self.hostname().to_string();
        let status = super::auth_status(&self.http, &hostname).await?;
        let world = status.get("world").and_then(Value::as_str)?.to_string();
        *self.shared.world_id.lock().await = Some(world.clone());
        Some(world)
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

    /// Complète, côté serveur, ce que le navigateur ajoute d'ordinaire dans
    /// `_preCreate` et que le serveur Foundry ne fait pas pour un client socket :
    /// - Scene (v14+) : au moins un niveau, sinon le canevas plante à
    ///   l'ouverture (`_determineInitialLevel`) ;
    /// - ChatMessage : `author`, requis par le schéma.
    async fn complete_create_data(&self, doc_type: &str, operation: &mut Value) {
        let Some(items) = operation.get_mut("data").and_then(Value::as_array_mut) else {
            return;
        };
        match doc_type {
            "Scene" if self.generation().await.unwrap_or(0) >= 14 => {
                for scene in items.iter_mut().filter_map(Value::as_object_mut) {
                    add_default_level(scene);
                }
            }
            "ChatMessage" => {
                let Some(uid) = self.user_id().await else {
                    return;
                };
                for msg in items.iter_mut().filter_map(Value::as_object_mut) {
                    if msg.get("author").is_none_or(Value::is_null) {
                        msg.insert("author".into(), json!(uid));
                    }
                }
            }
            _ => {}
        }
    }

    /// « modifyDocument » — le canal universel (get/create/update/delete).
    /// Toute écriture passe par ici : le garde-fou de script_guard d'abord.
    pub async fn modify_document(
        &self,
        doc_type: &str,
        action: &str,
        operation: Value,
    ) -> Result<Value> {
        self.guard_scripts(doc_type, action, &operation).await?;
        self.modify_document_raw(doc_type, action, operation).await
    }

    /// Refuse les écritures qui feraient exécuter du code au navigateur MJ en
    /// contournant les réglages du compagnon (voir script_guard).
    async fn guard_scripts(&self, doc_type: &str, action: &str, operation: &Value) -> Result<()> {
        use super::script_guard::{self, COMPANION, Need};
        if action != "create" && action != "update" {
            return Ok(());
        }
        let mut keys = Vec::new();
        if doc_type == "Setting" && action == "update" {
            for u in operation
                .get("updates")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(id) = u.get("_id").and_then(Value::as_str) {
                    let q = json!({ "_id": id })
                        .as_object()
                        .cloned()
                        .unwrap_or_default();
                    for d in self.get_collection_raw("Setting", q, false, None).await? {
                        if let Some(k) = d.get("key").and_then(Value::as_str) {
                            keys.push(k.to_string());
                        }
                    }
                }
            }
        }
        let needs = script_guard::inspect(doc_type, action, operation, &keys);
        if needs.is_empty() {
            return Ok(());
        }
        let mode = async |name: &str, default: &str| -> String {
            let q = json!({ "key": format!("{COMPANION}.{name}") })
                .as_object()
                .cloned()
                .unwrap_or_default();
            self.get_collection_raw("Setting", q, false, None)
                .await
                .ok()
                .and_then(|v| v.first()?.get("value")?.as_str().map(String::from))
                .and_then(|raw| serde_json::from_str::<String>(&raw).ok())
                .unwrap_or_else(|| default.to_string())
        };
        let script = mode("runScriptMode", "off").await;
        let macro_mode = mode("runMacroScriptMode", "confirm").await;
        for n in &needs {
            let ok = match n {
                Need::Forbidden(_) => false,
                Need::Script(_) => script == "auto",
                Need::MacroScript(_) => macro_mode == "auto",
            };
            if !ok {
                warn!("écriture refusée ({doc_type} {action}) : {}", n.reason());
                bail!("{}", script_guard::refusal(n, &script, &macro_mode));
            }
        }
        Ok(())
    }

    async fn modify_document_raw(
        &self,
        doc_type: &str,
        action: &str,
        mut operation: Value,
    ) -> Result<Value> {
        if action == "create" {
            self.complete_create_data(doc_type, &mut operation).await;
        }
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
        self.get_collection_raw(doc_type, query, index, pack).await
    }

    async fn get_collection_raw(
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
        let data = self.modify_document_raw(doc_type, "get", op).await?;
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
        let (http_s, _) = super::auth::schemes(&hostname);
        let url = format!("{http_s}://{host}{base}/upload");
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
            // v14 (build 368+) exige Origin sur les POST de formulaire
            .header(reqwest::header::ORIGIN, format!("{http_s}://{host}"))
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
        // checked_add : ce Duration peut venir d'un argument d'outil, et
        // `Instant + Duration` PANIQUE au débordement au lieu de saturer.
        // Les appelants bornent déjà, ceci est la ceinture de sécurité du
        // point de passage commun.
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(|| tokio::time::Instant::now() + std::time::Duration::from_secs(3600));
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(e)) => {
                    if e.seq > since_seq && predicate(&e) {
                        return Some(e);
                    }
                }
                // Lagged n'est PAS une fermeture : c'est une rafale qui a
                // débordé le buffer de 256. On continue avec le même
                // récepteur — l'événement attendu a pu passer dans le trou,
                // mais le timeout couvre déjà ce cas et se réabonner ne le
                // rattraperait pas davantage.
                Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                    warn!(skipped = n, "événements manqués en attendant une réponse");
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => rx = self.subscribe_events(),
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
        events: Mutex::new(std::collections::VecDeque::new()),
        outgoing: out_tx,
        event_tx,
        generation: Mutex::new(None),
        world_id: Mutex::new(None),
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
        // …ET vider les trames encore en file d'émission. Sans ça, une écriture
        // dont on vient d'abandonner l'ack juste au-dessus repartait quand même
        // sur la NOUVELLE socket : l'appelant voyait un échec, réessayait, et
        // le monde encaissait le doublon. Le Sender vit dans Shared, donc
        // try_recv ne rend jamais Disconnected — pas de boucle infinie.
        let mut dropped = 0usize;
        while out_rx.try_recv().is_ok() {
            dropped += 1;
        }
        if dropped > 0 {
            warn!(dropped, "trames sortantes abandonnées à la reconnexion");
        }
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
    // VecDeque : `Vec::remove(0)` recopiait les 300 entrées à chaque événement.
    buf.push_back(entry.clone());
    if buf.len() > EVENT_BUFFER_MAX {
        buf.pop_front();
    }
    drop(buf);
    let _ = shared.event_tx.send(entry);
}

/// Équivalent de `Scene#_preCreate` (v14) : sans niveau fourni, crée le niveau
/// par défaut à partir des anciens champs de fond (background, foreground…).
fn add_default_level(scene: &mut serde_json::Map<String, Value>) {
    if scene
        .get("levels")
        .and_then(Value::as_array)
        .is_some_and(|l| !l.is_empty())
    {
        return;
    }
    const DEFAULT_LEVEL: &str = "defaultLevel0000";
    let mut level = serde_json::Map::new();
    level.insert("_id".into(), json!(DEFAULT_LEVEL));
    level.insert(
        "name".into(),
        scene.get("name").cloned().unwrap_or(json!("Level")),
    );
    let background = scene.get("background").filter(|b| b.is_object()).cloned();
    let color = scene
        .get("backgroundColor")
        .filter(|c| !c.is_null())
        .cloned();
    if background.is_some() || color.is_some() {
        let mut bg = serde_json::Map::new();
        if let Some(c) = color {
            bg.insert("color".into(), c);
        }
        if let Some(b) = &background {
            if let Some(src) = b.get("src") {
                bg.insert("src".into(), src.clone());
            }
            for (from, to) in [("offsetX", "shiftX"), ("offsetY", "shiftY")] {
                if let Some(v) = b.get(from)
                    && !scene.contains_key(to)
                {
                    scene.insert(to.into(), v.clone());
                }
            }
        }
        level.insert("background".into(), Value::Object(bg));
    }
    if let Some(fg) = scene.get("foreground").filter(|f| !f.is_null()) {
        level.insert("foreground".into(), json!({ "src": fg }));
    }
    if let Some(top) = scene.get("foregroundElevation").filter(|e| !e.is_null()) {
        level.insert("elevation".into(), json!({ "top": top }));
    }
    if let Some(overlay) = scene
        .get("fog")
        .and_then(|f| f.get("overlay"))
        .filter(|o| !o.is_null())
    {
        level.insert("fog".into(), json!({ "src": overlay }));
    }
    scene.insert("levels".into(), json!([level]));
    scene.insert("initialLevel".into(), json!(DEFAULT_LEVEL));
}

#[cfg(test)]
mod tests {
    use super::add_default_level;
    use serde_json::{Value, json};

    fn scene(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn scene_v14_recoit_un_niveau_par_defaut() {
        let mut s = scene(
            json!({"name": "Cantina", "background": {"src": "maps/c.webp", "offsetX": 5},
                                 "backgroundColor": "#112233", "foregroundElevation": 20}),
        );
        add_default_level(&mut s);
        assert_eq!(s["initialLevel"], "defaultLevel0000");
        let level = &s["levels"][0];
        assert_eq!(level["_id"], "defaultLevel0000");
        assert_eq!(level["name"], "Cantina");
        assert_eq!(level["background"]["src"], "maps/c.webp");
        assert_eq!(level["background"]["color"], "#112233");
        assert_eq!(level["elevation"]["top"], 20);
        assert_eq!(s["shiftX"], 5);
    }

    #[test]
    fn scene_avec_niveaux_inchangee() {
        let mut s = scene(json!({"name": "S", "levels": [{"_id": "abc"}]}));
        add_default_level(&mut s);
        assert_eq!(s["levels"], json!([{"_id": "abc"}]));
        assert!(!s.contains_key("initialLevel"));
    }
}
