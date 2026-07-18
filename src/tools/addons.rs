//! Addons divers (hors « famille CC ») — outils MCP + délégations compagnon :
//!   · Monk's Active Tile Triggers (mat_*) : lister les tuiles-actions (serveur,
//!     documents) et les déclencher (client, TileDocument.trigger).
//!   · Sequencer (avancé) : effets entre tokens (attaques/projectiles) et sons
//!     (client — l'effet simple at-token/at-point est `client_play_effect`).

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::companion::call_companion;
use super::{str_arg, text_response};
use crate::foundry::documents::get_path;
use crate::mcp::McpState;

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("mat_list",
         "Monk's Active Tiles: list the trigger-tiles on a scene (default: active scene) — _id, name/label, trigger type(s), action count. Read-only (documents).",
         json!({"type":"object","properties":{
            "scene_id":{"type":"string"},"scene_name":{"type":"string"}}})),
        ("client_mat_trigger",
         "Monk's Active Tiles (companion): manually fire an active tile by its _id, optionally passing acting token _ids — runs its action chain (teleport, macros, scene change…).",
         json!({"type":"object","properties":{
            "tile":{"type":"string"},"tokens":{"type":"array","items":{"type":"string"}}},
            "required":["tile"]})),
        ("client_seq_between",
         "Sequencer (companion): play an effect FROM one token TO another (projectile/beam/attack). file + from_token + to_token (+ optional scale).",
         json!({"type":"object","properties":{
            "file":{"type":"string"},"from_token":{"type":"string"},"to_token":{"type":"string"},
            "scale":{"type":"number"}},"required":["file","from_token","to_token"]})),
        ("client_seq_sound",
         "Sequencer (companion): play a sound to all players via Sequencer (respects its ecosystem). file + optional volume.",
         json!({"type":"object","properties":{
            "file":{"type":"string"},"volume":{"type":"number"}},"required":["file"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "mat_list" => {
            let fields = vec!["_id".into(), "name".into(), "tiles".into()];
            let scene = match (str_arg(args, "scene_id"), str_arg(args, "scene_name")) {
                (Some(id), _) => {
                    foundry
                        .get_document("scenes", Some(&id), None, Some(&fields))
                        .await?
                }
                (None, Some(n)) => {
                    foundry
                        .get_document("scenes", None, Some(&n), Some(&fields))
                        .await?
                }
                _ => {
                    let w = json!({"active": true}).as_object().cloned().unwrap();
                    foundry
                        .get_documents("scenes", Some(&w), Some(&fields), 0, Some(1))
                        .await?
                        .into_iter()
                        .next()
                }
            }
            .ok_or_else(|| anyhow!("Scene not found (no active scene and none specified)"))?;
            let empty = vec![];
            let tiles = scene
                .get("tiles")
                .and_then(Value::as_array)
                .unwrap_or(&empty);
            let active: Vec<Value> = tiles
                .iter()
                .filter_map(|t| {
                    let mat = get_path(t, "flags.monks-active-tiles")?;
                    let actions = mat
                        .get("actions")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or(0);
                    let trigger = mat.get("trigger").cloned();
                    Some(json!({
                        "_id": t.get("_id"),
                        "name": mat.get("name").or_else(|| t.get("name")).cloned(),
                        "active": mat.get("active"),
                        "trigger": trigger,
                        "actions": actions,
                    }))
                })
                .collect();
            Ok(text_response(&json!({
                "scene": {"_id": scene["_id"], "name": scene["name"]},
                "count": active.len(), "tiles": active,
            })))
        }
        "client_mat_trigger" => {
            let tile = str_arg(args, "tile").ok_or_else(|| anyhow!("'tile' is required"))?;
            let mut a = json!({ "tileId": tile });
            if let Some(t) = args.get("tokens") {
                a["tokens"] = t.clone();
            }
            let r = call_companion(state, "mat_trigger", a, None, 20).await?;
            Ok(text_response(&r))
        }
        "client_seq_between" => {
            let file = str_arg(args, "file").ok_or_else(|| anyhow!("'file' is required"))?;
            let from =
                str_arg(args, "from_token").ok_or_else(|| anyhow!("'from_token' is required"))?;
            let to = str_arg(args, "to_token").ok_or_else(|| anyhow!("'to_token' is required"))?;
            let a = json!({ "file": file, "fromTokenId": from, "toTokenId": to,
                "scale": args.get("scale").cloned().unwrap_or(json!(1)) });
            let r = call_companion(state, "seq_between", a, None, 20).await?;
            Ok(text_response(&r))
        }
        "client_seq_sound" => {
            let file = str_arg(args, "file").ok_or_else(|| anyhow!("'file' is required"))?;
            let mut a = json!({ "file": file });
            if let Some(v) = args.get("volume") {
                a["volume"] = v.clone();
            }
            let r = call_companion(state, "seq_sound", a, None, 10).await?;
            Ok(text_response(&r))
        }
        other => bail!("Unknown tool: {other}"),
    }
}
