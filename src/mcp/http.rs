//! Transport « streamable HTTP » : POST = requêtes JSON-RPC (réponse JSON),
//! GET = flux SSE des notifications serveur, DELETE = fin de session,
//! /health = sonde. Secret dans le chemin (Claude Desktop sans en-têtes).

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::stream::Stream;
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::auth::{McpAuth, Scope};
use super::{McpState, Session, handle_message};

const SESSION_HEADER: &str = "mcp-session-id";

/// Le jeton vit dans le CHEMIN, pas dans un en-tête : plusieurs clients MCP
/// ne savent pas envoyer d'en-tête arbitraire. La contrepartie est réelle et
/// documentée dans SECURITY.md — l'URL doit être traitée comme un mot de passe
/// (journaux de proxy, historique de navigateur).
pub fn router(state: McpState, auth: McpAuth) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            "/{token_path}",
            get(sse_handler).post(post_handler).delete(delete_handler),
        )
        .layer(axum::middleware::from_fn_with_state(auth, authenticate))
        .with_state(state)
}

/// Vérifie le jeton du chemin avant tout traitement.
///
/// Le 404 sur jeton invalide est délibéré : un 401 confirmerait à un scanneur
/// que l'URL est la bonne et qu'il ne manque « que » le jeton.
async fn authenticate(
    axum::extract::State(auth): axum::extract::State<McpAuth>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = request.uri().path().trim_start_matches('/').to_string();
    let Some(token) = path.strip_prefix("mcp-") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match auth.verify(token) {
        Ok(scope) => {
            request.extensions_mut().insert(scope);
            next.run(request).await
        }
        Err(e) => {
            tracing::debug!("MCP token refused: {e:#}");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

async fn post_handler(
    State(state): State<McpState>,
    headers: HeaderMap,
    scope: axum::extract::Extension<Scope>,
    Json(message): Json<Value>,
) -> Response {
    // Un jeton en lecture seule contraint la session, quoi qu'en dise
    // l'environnement : la portée vient du jeton, pas de la configuration.
    let state = if *scope == Scope::ReadOnly {
        let mut s = state.clone();
        s.readonly = true;
        s
    } else {
        state
    };
    let state = &state;
    let incoming_sid = headers
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let is_initialize = message.get("method").and_then(Value::as_str) == Some("initialize");

    // initialize crée la session ; les autres requêtes la réutilisent.
    let sid = if is_initialize {
        let mut sessions = state.sessions.lock().await;
        // Plafond de pointe : le balayage borne la fuite dans le temps, ce
        // refus la borne en volume face à une boucle d'initialize.
        if sessions.len() >= super::MAX_SESSIONS {
            drop(sessions);
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "too many open MCP sessions — close some, or wait for the idle ones to be reclaimed",
            )
                .into_response();
        }
        let sid = Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::channel(64);
        sessions.insert(
            sid.clone(),
            Arc::new(Session {
                notify: tx,
                last_seen: std::sync::atomic::AtomicU64::new(super::now_ms()),
            }),
        );
        drop(sessions);
        // le récepteur attendra le GET SSE ; on le gare dans la session
        state
            .pending_rx
            .lock()
            .await
            .insert(sid.clone(), (rx, super::now_ms()));
        sid
    } else {
        match incoming_sid {
            Some(s) => s,
            None => return (StatusCode::BAD_REQUEST, "mcp-session-id missing").into_response(),
        }
    };

    // Marque l'activité : c'est ce qui empêche le balayage de couper une
    // session vivante mais sans flux SSE ouvert.
    if let Some(session) = state.sessions.lock().await.get(&sid) {
        session
            .last_seen
            .store(super::now_ms(), std::sync::atomic::Ordering::Relaxed);
    }

    match handle_message(state, &message).await {
        Some(reply) => (
            [
                (SESSION_HEADER, sid.as_str()),
                (header::CONTENT_TYPE.as_str(), "application/json"),
            ],
            Json(reply),
        )
            .into_response(),
        None => (StatusCode::ACCEPTED, [(SESSION_HEADER, sid.as_str())], "").into_response(),
    }
}

async fn sse_handler(
    State(state): State<McpState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, &'static str)> {
    // erreur en (StatusCode, &str) plutôt qu'en Response : clippy::result_large_err
    let sid = headers
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .ok_or((StatusCode::BAD_REQUEST, "mcp-session-id missing"))?;
    let (rx, _) = state.pending_rx.lock().await.remove(&sid).ok_or((
        StatusCode::NOT_FOUND,
        "unknown session, or its event stream is already open",
    ))?;

    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        let value = rx.recv().await?;
        let event = Event::default()
            .event("message")
            .data(serde_json::to_string(&value).unwrap_or_default());
        Some((Ok(event), rx))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn delete_handler(State(state): State<McpState>, headers: HeaderMap) -> StatusCode {
    if let Some(sid) = headers.get(SESSION_HEADER).and_then(|v| v.to_str().ok()) {
        state.sessions.lock().await.remove(sid);
        state.pending_rx.lock().await.remove(sid);
    }
    StatusCode::NO_CONTENT
}
