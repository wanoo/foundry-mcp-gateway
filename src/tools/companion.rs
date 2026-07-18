//! Outils `client_*` : délèguent au module Foundry « foundry-mcp-gateway-companion »
//! (côté navigateur) les actions que le protocole socket seul ne peut pas faire
//! — exécuter des macros, vrais jets système (Dice So Nice), caméra, sons,
//! notifications, API Campaign Codex, télémétrie des clients.
//!
//! Mécanique : on émet une commande sur le canal `module.foundry-mcp-gateway-companion`
//! et on attend la réponse dans le buffer d'événements. Si aucun module n'est
//! installé/actif, la commande expire avec un message explicite.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::time::Duration;

use super::{str_arg, text_response};
use crate::mcp::McpState;

const CHANNEL: &str = "module.foundry-mcp-gateway-companion";

/// Convention `targets` (commandes de scène) : "all" (défaut), "gm", "players",
/// ou un tableau d'_id d'utilisateurs.
// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("client_status",
         "Check the companion module: is foundry-mcp-gateway-companion installed and active? Returns its version, the responding GM, and which optional deps (Dice So Nice, Campaign Codex, Sequencer) are available.",
         json!({"type":"object","properties":{}})),
        ("client_run_macro",
         "Run a Foundry macro by _id or name on the GM client (returns its return value). Unlocks anything scriptable that the socket API cannot do.",
         json!({"type":"object","properties":{
            "macro":{"type":"string"},"scope":{"type":"object","additionalProperties":true}},
            "required":["macro"]})),
        ("client_run_script",
         "Run arbitrary JavaScript on the GM client (async, receives game/canvas/ui). DISABLED by default — the GM must enable it in the module settings.",
         json!({"type":"object","properties":{"code":{"type":"string"}},"required":["code"]})),
        ("client_roll_pool_native",
         "starwarsffg: roll a dice pool with the REAL FFG engine (native chat card + Dice So Nice 3D on the table). Pass the pool (compute it with roll_actor_skill first if you want a sheet-derived pool).",
         json!({"type":"object","properties":{
            "pool":{"type":"object","additionalProperties":true,"description":"{ability,proficiency,difficulty,challenge,boost,setback,force}"},
            "description":{"type":"string"},"actor":{"type":"string","description":"actor _id for the chat speaker"}},
            "required":["pool"]})),
        ("client_pan_camera",
         "Pan (and zoom) the camera on the targeted clients — 'look here'. Give x/y, or a token _id to center on. targets: all (default) / gm / players / [userId].",
         json!({"type":"object","properties":{
            "x":{"type":"number"},"y":{"type":"number"},"scale":{"type":"number"},
            "token":{"type":"string"},"targets":{}}})),
        ("client_ping",
         "Ping a point on the map for the targeted clients. x/y or a token _id. targets: all (default) / gm / players / [userId].",
         json!({"type":"object","properties":{
            "x":{"type":"number"},"y":{"type":"number"},"token":{"type":"string"},"targets":{}}})),
        ("client_play_sound",
         "Play a one-shot sound on the targeted clients (a dramatic stinger). targets: all (default) / gm / players / [userId].",
         json!({"type":"object","properties":{
            "src":{"type":"string"},"volume":{"type":"number"},"loop":{"type":"boolean"},"targets":{}},
            "required":["src"]})),
        ("client_notify",
         "Show a UI notification on the targeted clients. type: info/warn/error. targets: all (default) / gm / players / [userId].",
         json!({"type":"object","properties":{
            "message":{"type":"string"},"type":{"type":"string"},"targets":{}},
            "required":["message"]})),
        ("client_show_document",
         "Open a document sheet (by uuid) on the targeted clients. targets: all (default) / gm / players / [userId].",
         json!({"type":"object","properties":{"uuid":{"type":"string"},"targets":{}},"required":["uuid"]})),
        ("client_play_effect",
         "Play a Sequencer visual effect (if the Sequencer module is active). At a token _id or x/y.",
         json!({"type":"object","properties":{
            "file":{"type":"string"},"token":{"type":"string"},
            "x":{"type":"number"},"y":{"type":"number"},"scale":{"type":"number"}},
            "required":["file"]})),
        ("client_get_state",
         "Client telemetry: active users, each one's viewed scene and assigned character, and the GM's current scene. (Live selections/targets stream via get_events as selection/target events.)",
         json!({"type":"object","properties":{}})),
    ]
}

pub fn handles(name: &str) -> bool {
    name.starts_with("client_")
}

/// Émet une commande sur le canal du module et attend la réponse (reply == id).
pub async fn call_companion(
    state: &McpState,
    cmd: &str,
    args: Value,
    targets: Option<Value>,
    timeout_secs: u64,
) -> Result<Value> {
    let id = uuid::Uuid::new_v4().to_string();
    let mut command = json!({ "mcp": true, "cmd": cmd, "id": id, "args": args });
    if let Some(t) = targets {
        command["targets"] = t;
    }
    let since = state.foundry.event_seq();
    state.foundry.emit(CHANNEL, &[command]).await?;

    let id_for_pred = id.clone();
    let matched = state
        .foundry
        .wait_for_event(since, Duration::from_secs(timeout_secs), move |e| {
            e.event == CHANNEL
                && e.args
                    .first()
                    .and_then(|a| a.get("reply"))
                    .map(|r| r == &json!(id_for_pred))
                    .unwrap_or(false)
        })
        .await;

    let event = matched.ok_or_else(|| {
        anyhow!(
            "no companion module responded to '{cmd}' — is foundry-mcp-gateway-companion installed and active, with a GM browser connected?"
        )
    })?;
    let payload = event.args.into_iter().next().unwrap_or(Value::Null);
    if payload.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(payload.get("result").cloned().unwrap_or(Value::Null))
    } else {
        bail!(
            "companion error: {}",
            payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        )
    }
}

/// Résout un argument token/actor : renvoie l'_id tel quel (les outils client
/// attendent des _ids ; utiliser list_tokens/get_actor pour les obtenir).
fn passthrough_id(args: &Value, key: &str) -> Option<Value> {
    str_arg(args, key).map(Value::String)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let targets = args.get("targets").cloned();
    let result = match name {
        "client_status" => call_companion(state, "ping_module", json!({}), None, 8).await?,
        "client_run_macro" => {
            let macro_ref = str_arg(args, "macro").ok_or_else(|| anyhow!("'macro' is required"))?;
            let scope = args.get("scope").cloned().unwrap_or(json!({}));
            call_companion(
                state,
                "run_macro",
                json!({ "macro": macro_ref, "scope": scope }),
                None,
                30,
            )
            .await?
        }
        "client_run_script" => {
            let code = str_arg(args, "code").ok_or_else(|| anyhow!("'code' is required"))?;
            call_companion(state, "run_script", json!({ "code": code }), None, 30).await?
        }
        "client_roll_pool_native" => {
            let pool = args
                .get("pool")
                .cloned()
                .ok_or_else(|| anyhow!("'pool' is required"))?;
            let mut a = json!({ "pool": pool });
            if let Some(d) = str_arg(args, "description") {
                a["description"] = json!(d);
            }
            if let Some(actor) = passthrough_id(args, "actor") {
                a["actorId"] = actor;
            }
            call_companion(state, "roll_pool_native", a, None, 30).await?
        }
        "client_pan_camera" => {
            let mut a = json!({});
            for k in ["x", "y", "scale"] {
                if let Some(v) = args.get(k) {
                    a[k] = v.clone();
                }
            }
            if let Some(t) = passthrough_id(args, "token") {
                a["tokenId"] = t;
            }
            call_companion(state, "pan", a, targets, 10).await?
        }
        "client_ping" => {
            let mut a = json!({});
            for k in ["x", "y"] {
                if let Some(v) = args.get(k) {
                    a[k] = v.clone();
                }
            }
            if let Some(t) = passthrough_id(args, "token") {
                a["tokenId"] = t;
            }
            call_companion(state, "ping_at", a, targets, 10).await?
        }
        "client_play_sound" => {
            let src = str_arg(args, "src").ok_or_else(|| anyhow!("'src' is required"))?;
            let mut a = json!({ "src": src });
            if let Some(v) = args.get("volume") {
                a["volume"] = v.clone();
            }
            if let Some(v) = args.get("loop") {
                a["loop"] = v.clone();
            }
            call_companion(state, "play_sound", a, targets, 10).await?
        }
        "client_notify" => {
            let message =
                str_arg(args, "message").ok_or_else(|| anyhow!("'message' is required"))?;
            let mut a = json!({ "message": message });
            if let Some(t) = str_arg(args, "type") {
                a["type"] = json!(t);
            }
            call_companion(state, "notify", a, targets, 10).await?
        }
        "client_show_document" => {
            let uuid = str_arg(args, "uuid").ok_or_else(|| anyhow!("'uuid' is required"))?;
            call_companion(state, "show_document", json!({ "uuid": uuid }), targets, 10).await?
        }
        "client_play_effect" => {
            let file = str_arg(args, "file").ok_or_else(|| anyhow!("'file' is required"))?;
            let mut a =
                json!({ "file": file, "scale": args.get("scale").cloned().unwrap_or(json!(1)) });
            if let Some(t) = passthrough_id(args, "token") {
                a["atTokenId"] = t;
            }
            for k in ["x", "y"] {
                if let Some(v) = args.get(k) {
                    a[k] = v.clone();
                }
            }
            call_companion(state, "play_effect", a, None, 20).await?
        }
        "client_get_state" => {
            call_companion(state, "get_client_state", json!({}), None, 10).await?
        }
        other => bail!("Unknown tool: {other}"),
    };
    Ok(text_response(&result))
}
