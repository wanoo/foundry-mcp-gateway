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

use super::{McpState, Session, handle_message};

const SESSION_HEADER: &str = "mcp-session-id";

pub fn router(state: McpState, secret: &str) -> Router {
    let mcp_path = format!("/mcp-{secret}");
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            &mcp_path,
            get(sse_handler).post(post_handler).delete(delete_handler),
        )
        .with_state(state)
}

async fn post_handler(
    State(state): State<McpState>,
    headers: HeaderMap,
    Json(message): Json<Value>,
) -> Response {
    let incoming_sid = headers
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let is_initialize = message.get("method").and_then(Value::as_str) == Some("initialize");

    // initialize crée la session ; les autres requêtes la réutilisent.
    let sid = if is_initialize {
        let sid = Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::channel(64);
        state
            .sessions
            .lock()
            .await
            .insert(sid.clone(), Arc::new(Session { notify: tx }));
        // le récepteur attendra le GET SSE ; on le gare dans la session
        PENDING_RX.lock().await.insert(sid.clone(), rx);
        sid
    } else {
        match incoming_sid {
            Some(s) => s,
            None => return (StatusCode::BAD_REQUEST, "mcp-session-id manquant").into_response(),
        }
    };

    match handle_message(&state, &message).await {
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

/// Récepteurs SSE en attente d'ouverture du flux GET (un par session).
static PENDING_RX: std::sync::LazyLock<
    tokio::sync::Mutex<std::collections::HashMap<String, mpsc::Receiver<Value>>>,
> = std::sync::LazyLock::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

async fn sse_handler(
    State(_state): State<McpState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Response> {
    let sid = headers
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "mcp-session-id manquant").into_response())?;
    let rx = PENDING_RX.lock().await.remove(&sid).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "session inconnue ou flux déjà ouvert",
        )
            .into_response()
    })?;

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
        PENDING_RX.lock().await.remove(sid);
    }
    StatusCode::NO_CONTENT
}
