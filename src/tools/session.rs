//! Outils de séance : diffusion aux joueurs, scènes, tokens, conditions,
//! combat, playlists, tables — port 1:1 des comportements validés en TS.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};

use super::{str_arg, text_response};
use crate::mcp::McpState;

/// Statuts core Foundry v13 (CONFIG.statusEffects) : id → (nom, icône).
pub const CORE_STATUS_EFFECTS: [(&str, &str, &str); 27] = [
    ("dead", "Dead", "icons/svg/skull.svg"),
    ("unconscious", "Unconscious", "icons/svg/unconscious.svg"),
    ("sleep", "Asleep", "icons/svg/sleep.svg"),
    ("stun", "Stunned", "icons/svg/daze.svg"),
    ("prone", "Prone", "icons/svg/falling.svg"),
    ("restrain", "Restrained", "icons/svg/net.svg"),
    ("paralysis", "Paralyzed", "icons/svg/paralysis.svg"),
    ("fly", "Flying", "icons/svg/wing.svg"),
    ("blind", "Blind", "icons/svg/blind.svg"),
    ("deaf", "Deaf", "icons/svg/deaf.svg"),
    ("silence", "Silenced", "icons/svg/silenced.svg"),
    ("fear", "Frightened", "icons/svg/terror.svg"),
    ("burning", "Burning", "icons/svg/fire.svg"),
    ("frozen", "Frozen", "icons/svg/frozen.svg"),
    ("shock", "Shocked", "icons/svg/lightning.svg"),
    ("corrode", "Corroding", "icons/svg/acid.svg"),
    ("bleeding", "Bleeding", "icons/svg/blood.svg"),
    ("disease", "Diseased", "icons/svg/biohazard.svg"),
    ("poison", "Poisoned", "icons/svg/poison.svg"),
    ("curse", "Cursed", "icons/svg/sun.svg"),
    ("regen", "Regenerating", "icons/svg/regen.svg"),
    ("degen", "Degenerating", "icons/svg/degen.svg"),
    ("invisible", "Invisible", "icons/svg/invisible.svg"),
    ("target", "Targeted", "icons/svg/target.svg"),
    ("eye", "Marked", "icons/svg/eye.svg"),
    ("bless", "Blessed", "icons/svg/angel.svg"),
    ("upgrade", "Upgraded", "icons/svg/upgrade.svg"),
];

pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("show_journal_to_players",
         "Show a JournalEntry to connected players (GM 'Show to Players'). force overrides permissions.",
         json!({"type":"object","properties":{
            "_id":{"type":"string"},"name":{"type":"string"},
            "uuid":{"type":"string","description":"Full uuid (alternative), e.g. JournalEntry.abc or a page uuid"},
            "force":{"type":"boolean"},"users":{"type":"array","items":{"type":"string"}}}})),
        ("share_image",
         "Display an image fullscreen to connected players (ImagePopout).",
         json!({"type":"object","properties":{
            "image":{"type":"string"},"title":{"type":"string"},"caption":{"type":"string"},
            "users":{"type":"array","items":{"type":"string"}},"show_title":{"type":"boolean"}},
            "required":["image"]})),
        ("toggle_pause",
         "Pause or unpause the game for everyone (GM only).",
         json!({"type":"object","properties":{"paused":{"type":"boolean"}},"required":["paused"]})),
        ("activate_scene",
         "Set a scene as active (what players see); pull_users to also pull everyone.",
         json!({"type":"object","properties":{
            "_id":{"type":"string"},"name":{"type":"string"},"pull_users":{"type":"boolean"}}})),
        ("pull_users_to_scene",
         "Pull users to a scene without activating it (default: everyone except this bot).",
         json!({"type":"object","properties":{
            "_id":{"type":"string"},"name":{"type":"string"},
            "users":{"type":"array","items":{"type":"string"}}}})),
        ("list_tokens",
         "Tokens of a scene (default: active scene) — position, actor, visibility.",
         json!({"type":"object","properties":{"scene_id":{"type":"string"},"scene_name":{"type":"string"}}})),
        ("move_token",
         "Move a token (pixels, top-left origin; a grid square is usually 100px).",
         json!({"type":"object","properties":{
            "token":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"},
            "elevation":{"type":"number"},"scene_id":{"type":"string"},"scene_name":{"type":"string"}},
            "required":["token"]})),
        ("update_token",
         "Update arbitrary token fields (hidden, disposition, name...). Use move_token for position.",
         json!({"type":"object","properties":{
            "token":{"type":"string"},"updates":{"type":"object","additionalProperties":true},
            "scene_id":{"type":"string"},"scene_name":{"type":"string"}},
            "required":["token","updates"]})),
        ("place_token",
         "Place an actor's token on a scene using its prototype token.",
         json!({"type":"object","properties":{
            "actor":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"},
            "hidden":{"type":"boolean"},"scene_id":{"type":"string"},"scene_name":{"type":"string"}},
            "required":["actor","x","y"]})),
        ("toggle_actor_condition",
         "Add/remove a core status condition (ActiveEffect) on a WORLD actor. Conditions: dead, stun, prone, bleeding, fear… Idempotent.",
         json!({"type":"object","properties":{
            "_id":{"type":"string"},"name":{"type":"string"},
            "condition":{"type":"string"},"active":{"type":"boolean"}},
            "required":["condition","active"]})),
        ("manage_combat",
         "Combat encounters: create (combatants from scene tokens), add_combatants, set_initiative, start, next_turn, next_round, status, end. Order = initiative desc.",
         json!({"type":"object","properties":{
            "action":{"type":"string","enum":["create","add_combatants","set_initiative","start","next_turn","next_round","status","end"]},
            "combat_id":{"type":"string"},"scene_id":{"type":"string"},
            "tokens":{"type":"array","items":{"type":"string"}},
            "combatant":{"type":"string"},"initiative":{"type":"number"}},
            "required":["action"]})),
        ("control_playlist",
         "Play or stop a playlist (or one sound). Modes: sequential/shuffle play one, simultaneous plays all, soundboard needs a sound.",
         json!({"type":"object","properties":{
            "playlist":{"type":"string"},"action":{"type":"string","enum":["play","stop"]},
            "sound":{"type":"string"}},"required":["playlist","action"]})),
        ("draw_from_table",
         "Draw from a RollTable server-side (NdM±k formula, range pick, modifier, multi-rolls, chat post).",
         json!({"type":"object","properties":{
            "table":{"type":"string"},"modifier":{"type":"number"},"rolls":{"type":"number"},
            "post":{"type":"boolean"},"whisper_users":{"type":"array","items":{"type":"string"}}},
            "required":["table"]})),
    ]
}

async fn resolve_scene(state: &McpState, args: &Value) -> Result<Value> {
    let fields = vec!["_id".into(), "name".into(), "tokens".into()];
    let scene = match (str_arg(args, "scene_id"), str_arg(args, "scene_name")) {
        (Some(id), _) => {
            state
                .foundry
                .get_document("scenes", Some(&id), None, Some(&fields))
                .await?
        }
        (None, Some(n)) => {
            state
                .foundry
                .get_document("scenes", None, Some(&n), Some(&fields))
                .await?
        }
        _ => {
            let w = json!({"active": true}).as_object().cloned().unwrap();
            state
                .foundry
                .get_documents("scenes", Some(&w), Some(&fields), 0, Some(1))
                .await?
                .into_iter()
                .next()
        }
    };
    scene.ok_or_else(|| anyhow!("Scene not found (no active scene and none specified)"))
}

async fn find_actor(
    state: &McpState,
    args: &Value,
    key_id: &str,
    key_name: &str,
    fields: &[String],
) -> Result<Value> {
    let doc = match (str_arg(args, key_id), str_arg(args, key_name)) {
        (Some(i), _) => {
            state
                .foundry
                .find_document("actors", &i, Some(fields))
                .await?
        }
        (None, Some(n)) => {
            state
                .foundry
                .get_document("actors", None, Some(&n), Some(fields))
                .await?
        }
        _ => bail!("Must provide one of: {key_id} or {key_name}"),
    };
    doc.ok_or_else(|| anyhow!("Actor not found"))
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "show_journal_to_players" => {
            let uuid = match str_arg(args, "uuid") {
                Some(u) => u,
                None => {
                    let id = str_arg(args, "_id");
                    let jname = str_arg(args, "name");
                    if id.is_none() && jname.is_none() {
                        bail!("Must provide one of: uuid, _id, or name");
                    }
                    let fields = vec!["_id".into(), "name".into()];
                    let doc = foundry
                        .get_document("journal", id.as_deref(), jname.as_deref(), Some(&fields))
                        .await?
                        .ok_or_else(|| anyhow!("JournalEntry not found"))?;
                    format!("JournalEntry.{}", doc["_id"].as_str().unwrap_or(""))
                }
            };
            let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
            let users = args.get("users").cloned().unwrap_or(json!([]));
            foundry
                .emit_with_ack(
                    "showEntry",
                    &[json!(uuid), json!({"force": force, "users": users})],
                )
                .await?;
            Ok(text_response(&json!({"shown": uuid, "force": force})))
        }
        "share_image" => {
            let image = str_arg(args, "image").ok_or_else(|| anyhow!("'image' is required"))?;
            let mut cfg = json!({
                "image": image,
                "users": args.get("users").cloned().unwrap_or(json!([])),
                "showTitle": args.get("show_title").and_then(Value::as_bool).unwrap_or(true),
            });
            if let Some(t) = str_arg(args, "title") {
                cfg["title"] = json!(t);
            }
            if let Some(c) = str_arg(args, "caption") {
                cfg["caption"] = json!(c);
            }
            foundry.emit("shareImage", &[cfg]).await?;
            Ok(text_response(&json!({"shared": image})))
        }
        "toggle_pause" => {
            let paused = args
                .get("paused")
                .and_then(Value::as_bool)
                .ok_or_else(|| anyhow!("'paused' is required"))?;
            let user_id = foundry.user_id().await;
            foundry
                .emit(
                    "pause",
                    &[json!(paused), json!({"broadcast": true, "userId": user_id})],
                )
                .await?;
            Ok(text_response(&json!({"paused": paused})))
        }
        "activate_scene" | "pull_users_to_scene" => {
            let scene = resolve_scene_by_ident(state, args).await?;
            let scene_id = scene["_id"].as_str().unwrap_or("").to_string();
            if name == "activate_scene" {
                foundry
                    .modify_document(
                        "Scene",
                        "update",
                        json!({
                            "action": "update", "diff": false, "recursive": true, "render": true,
                            "updates": [{"_id": scene_id, "active": true}],
                        }),
                    )
                    .await?;
            }
            let mut pulled: Vec<String> = Vec::new();
            let want_pull = name == "pull_users_to_scene"
                || args
                    .get("pull_users")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            if want_pull {
                pulled = args
                    .get("users")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                if pulled.is_empty() {
                    let fields = vec!["_id".into()];
                    let users = foundry
                        .get_documents("users", None, Some(&fields), 0, None)
                        .await?;
                    let self_id = foundry.user_id().await.unwrap_or_default();
                    pulled = users
                        .iter()
                        .filter_map(|u| u["_id"].as_str())
                        .filter(|id| *id != self_id)
                        .map(String::from)
                        .collect();
                }
                for uid in &pulled {
                    foundry
                        .emit("pullToScene", &[json!(scene_id), json!(uid)])
                        .await?;
                }
            }
            Ok(text_response(&json!({
                "scene": {"_id": scene_id, "name": scene["name"]},
                "activated": name == "activate_scene",
                "pulledUsers": pulled,
            })))
        }
        "list_tokens" | "move_token" | "update_token" => {
            let scene = resolve_scene(state, args).await?;
            let empty = vec![];
            let tokens = scene
                .get("tokens")
                .and_then(Value::as_array)
                .unwrap_or(&empty);
            if name == "list_tokens" {
                let list: Vec<Value> = tokens.iter().map(|t| json!({
                    "_id": t["_id"], "name": t["name"], "x": t["x"], "y": t["y"],
                    "elevation": t["elevation"], "hidden": t["hidden"],
                    "actorId": t["actorId"], "actorLink": t["actorLink"], "disposition": t["disposition"],
                })).collect();
                return Ok(text_response(&json!({
                    "scene": {"_id": scene["_id"], "name": scene["name"]}, "tokens": list,
                })));
            }
            let tok_arg = str_arg(args, "token").ok_or_else(|| anyhow!("'token' is required"))?;
            let token = tokens
                .iter()
                .find(|t| t["_id"] == json!(tok_arg) || t["name"] == json!(tok_arg))
                .ok_or_else(|| anyhow!("Token not found on scene {}: {tok_arg}", scene["name"]))?;
            let mut updates = Map::new();
            if name == "move_token" {
                for key in ["x", "y", "elevation"] {
                    if let Some(v) = args.get(key)
                        && !v.is_null()
                    {
                        updates.insert(key.into(), v.clone());
                    }
                }
                if updates.is_empty() {
                    bail!("Provide at least one of: x, y, elevation");
                }
            } else {
                updates = args
                    .get("updates")
                    .and_then(Value::as_object)
                    .cloned()
                    .filter(|u| !u.is_empty())
                    .ok_or_else(|| anyhow!("'updates' must not be empty"))?;
            }
            let mut update_doc = Value::Object(updates.clone());
            update_doc["_id"] = token["_id"].clone();
            let result = foundry
                .modify_document(
                    "Token",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "parentUuid": format!("Scene.{}", scene["_id"].as_str().unwrap_or("")),
                        "updates": [update_doc],
                    }),
                )
                .await?;
            Ok(text_response(&json!({
                "scene": {"_id": scene["_id"], "name": scene["name"]},
                "token": {"_id": token["_id"], "name": token["name"]},
                "applied": updates, "result": result,
            })))
        }
        "place_token" => {
            let actor_arg = str_arg(args, "actor").ok_or_else(|| anyhow!("'actor' is required"))?;
            let x = args
                .get("x")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("'x' is required"))?;
            let y = args
                .get("y")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("'y' is required"))?;
            let fields = vec!["_id".into(), "name".into(), "prototypeToken".into()];
            let actor = foundry
                .find_document("actors", &actor_arg, Some(&fields))
                .await?
                .ok_or_else(|| anyhow!("Actor not found: {actor_arg}"))?;
            let scene = resolve_scene(state, args).await?;
            let mut token = actor.get("prototypeToken").cloned().unwrap_or(json!({}));
            if token
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty()
            {
                token["name"] = actor["name"].clone();
            }
            token["actorId"] = actor["_id"].clone();
            token["x"] = json!(x);
            token["y"] = json!(y);
            if let Some(h) = args.get("hidden").and_then(Value::as_bool) {
                token["hidden"] = json!(h);
            }
            let result = foundry.modify_document("Token", "create", json!({
                "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                "parentUuid": format!("Scene.{}", scene["_id"].as_str().unwrap_or("")),
                "data": [token],
            })).await?;
            Ok(text_response(&json!({
                "actor": {"_id": actor["_id"], "name": actor["name"]},
                "scene": {"_id": scene["_id"], "name": scene["name"]},
                "x": x, "y": y, "result": result,
            })))
        }
        "toggle_actor_condition" => {
            let condition =
                str_arg(args, "condition").ok_or_else(|| anyhow!("'condition' is required"))?;
            let active = args
                .get("active")
                .and_then(Value::as_bool)
                .ok_or_else(|| anyhow!("'active' is required"))?;
            let status = CORE_STATUS_EFFECTS
                .iter()
                .find(|(id, _, _)| *id == condition)
                .ok_or_else(|| {
                    anyhow!(
                        "Unknown condition '{condition}'. Available: {}",
                        CORE_STATUS_EFFECTS
                            .iter()
                            .map(|(i, _, _)| *i)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            let fields = vec!["_id".into(), "name".into(), "effects".into()];
            let actor = find_actor(state, args, "_id", "name", &fields).await?;
            let empty = vec![];
            let effects = actor
                .get("effects")
                .and_then(Value::as_array)
                .unwrap_or(&empty);
            let existing: Vec<&Value> = effects
                .iter()
                .filter(|e| {
                    e.get("statuses")
                        .and_then(Value::as_array)
                        .map(|s| s.contains(&json!(condition)))
                        .unwrap_or(false)
                })
                .collect();
            let parent = format!("Actor.{}", actor["_id"].as_str().unwrap_or(""));
            if active {
                if !existing.is_empty() {
                    return Ok(text_response(&json!({
                        "actor": {"_id": actor["_id"], "name": actor["name"]},
                        "condition": condition, "unchanged": "already active",
                    })));
                }
                let result = foundry.modify_document("ActiveEffect", "create", json!({
                    "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                    "parentUuid": parent,
                    "data": [{"name": status.1, "img": status.2, "statuses": [condition]}],
                })).await?;
                Ok(text_response(&json!({
                    "actor": {"_id": actor["_id"], "name": actor["name"]},
                    "condition": condition, "added": true, "result": result,
                })))
            } else {
                if existing.is_empty() {
                    return Ok(text_response(&json!({
                        "actor": {"_id": actor["_id"], "name": actor["name"]},
                        "condition": condition, "unchanged": "not active",
                    })));
                }
                let ids: Vec<Value> = existing.iter().map(|e| e["_id"].clone()).collect();
                let removed = ids.len();
                let result = foundry.modify_document("ActiveEffect", "delete", json!({
                    "action": "delete", "broadcast": false, "parentUuid": parent, "ids": ids,
                })).await?;
                Ok(text_response(&json!({
                    "actor": {"_id": actor["_id"], "name": actor["name"]},
                    "condition": condition, "removed": removed, "result": result,
                })))
            }
        }
        "manage_combat" => manage_combat(state, args).await,
        "control_playlist" => control_playlist(state, args).await,
        "draw_from_table" => draw_from_table(state, args).await,
        other => bail!("Unknown tool: {other}"),
    }
}

/// Résolution d'une scène par _id/name (pour activate/pull, champs légers).
async fn resolve_scene_by_ident(state: &McpState, args: &Value) -> Result<Value> {
    let fields = vec!["_id".into(), "name".into()];
    let doc = match (str_arg(args, "_id"), str_arg(args, "name")) {
        (Some(i), _) => {
            state
                .foundry
                .get_document("scenes", Some(&i), None, Some(&fields))
                .await?
        }
        (None, Some(n)) => {
            state
                .foundry
                .get_document("scenes", None, Some(&n), Some(&fields))
                .await?
        }
        _ => bail!("Must provide one of: _id or name"),
    };
    doc.ok_or_else(|| anyhow!("Scene not found"))
}

async fn scene_tokens(state: &McpState, scene_id: Option<String>) -> Result<Value> {
    let fields = vec!["_id".into(), "name".into(), "tokens".into()];
    let scene = match scene_id {
        Some(id) => {
            state
                .foundry
                .get_document("scenes", Some(&id), None, Some(&fields))
                .await?
        }
        None => {
            let w = json!({"active": true}).as_object().cloned().unwrap();
            state
                .foundry
                .get_documents("scenes", Some(&w), Some(&fields), 0, Some(1))
                .await?
                .into_iter()
                .next()
        }
    };
    scene.ok_or_else(|| anyhow!("Scene not found (no active scene and none specified)"))
}

/// Résolution des noms de combattants via les tokens de la scène.
async fn with_names(state: &McpState, combat: &Value, order: Vec<Value>) -> Result<Vec<Value>> {
    if order.is_empty()
        || order
            .iter()
            .all(|c| c.get("name").is_some_and(|n| !n.is_null()))
    {
        return Ok(order);
    }
    let scene_id = combat
        .get("scene")
        .and_then(Value::as_str)
        .or_else(|| {
            order
                .first()
                .and_then(|c| c.get("sceneId"))
                .and_then(Value::as_str)
        })
        .map(String::from);
    let Some(sid) = scene_id else {
        return Ok(order);
    };
    let fields = vec!["_id".into(), "tokens".into()];
    let scene = state
        .foundry
        .get_document("scenes", Some(&sid), None, Some(&fields))
        .await?;
    let empty = vec![];
    let tokens = scene
        .as_ref()
        .and_then(|s| s.get("tokens"))
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    Ok(order
        .into_iter()
        .map(|mut c| {
            if c.get("name").is_none_or(Value::is_null) {
                let tname = tokens
                    .iter()
                    .find(|t| t["_id"] == c["tokenId"])
                    .and_then(|t| t.get("name"))
                    .cloned()
                    .unwrap_or(Value::Null);
                c["name"] = tname;
            }
            c
        })
        .collect())
}

async fn manage_combat(state: &McpState, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    let action = str_arg(args, "action").ok_or_else(|| anyhow!("'action' is required"))?;

    let sorted = |combat: &Value| -> Vec<Value> {
        let mut list = combat
            .get("combatants")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        list.sort_by(|a, b| {
            let ia = a
                .get("initiative")
                .and_then(Value::as_f64)
                .unwrap_or(f64::NEG_INFINITY);
            let ib = b
                .get("initiative")
                .and_then(Value::as_f64)
                .unwrap_or(f64::NEG_INFINITY);
            ib.partial_cmp(&ia).unwrap_or(std::cmp::Ordering::Equal)
        });
        list
    };

    if action == "create" {
        let scene = scene_tokens(state, str_arg(args, "scene_id")).await?;
        let empty = vec![];
        let all = scene
            .get("tokens")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        let wanted: Option<Vec<String>> = args.get("tokens").and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        });
        let tokens: Vec<&Value> = all
            .iter()
            .filter(|t| match &wanted {
                Some(w) => w
                    .iter()
                    .any(|x| t["_id"] == json!(x) || t["name"] == json!(x)),
                None => t.get("actorId").is_some_and(|a| !a.is_null()),
            })
            .collect();
        let created = foundry
            .modify_document(
                "Combat",
                "create",
                json!({
                    "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                    "data": [{"scene": scene["_id"], "active": true}],
                }),
            )
            .await?;
        let combat_id = created
            .pointer("/result/0/_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Combat creation gave no _id: {created}"))?
            .to_string();
        if !tokens.is_empty() {
            let data: Vec<Value> = tokens
                .iter()
                .map(|t| {
                    json!({
                        "tokenId": t["_id"], "sceneId": scene["_id"], "actorId": t["actorId"],
                        "hidden": t.get("hidden").cloned().unwrap_or(json!(false)),
                    })
                })
                .collect();
            foundry.modify_document("Combatant", "create", json!({
                "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                "parentUuid": format!("Combat.{combat_id}"), "data": data,
            })).await?;
        }
        return Ok(text_response(&json!({
            "combat": combat_id,
            "scene": {"_id": scene["_id"], "name": scene["name"]},
            "combatants": tokens.iter().map(|t| t["name"].clone()).collect::<Vec<_>>(),
        })));
    }

    // Combat courant : combat_id fourni, sinon actif, sinon premier.
    let combat = match str_arg(args, "combat_id") {
        Some(id) => {
            foundry
                .get_document("combats", Some(&id), None, None)
                .await?
        }
        None => {
            let all = foundry
                .get_documents("combats", None, None, 0, None)
                .await?;
            all.iter()
                .find(|c| c.get("active") == Some(&json!(true)))
                .cloned()
                .or_else(|| all.into_iter().next())
        }
    }
    .ok_or_else(|| anyhow!("No combat found (create one first)"))?;
    let combat_id = combat["_id"].as_str().unwrap_or("").to_string();

    match action.as_str() {
        "add_combatants" => {
            let scene = scene_tokens(
                state,
                combat
                    .get("scene")
                    .and_then(Value::as_str)
                    .map(String::from),
            )
            .await?;
            let empty = vec![];
            let all = scene
                .get("tokens")
                .and_then(Value::as_array)
                .unwrap_or(&empty);
            let wanted: Vec<String> = args
                .get("tokens")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .filter(|v: &Vec<String>| !v.is_empty())
                .ok_or_else(|| anyhow!("'tokens' is required"))?;
            let tokens: Vec<&Value> = all
                .iter()
                .filter(|t| {
                    wanted
                        .iter()
                        .any(|x| t["_id"] == json!(x) || t["name"] == json!(x))
                })
                .collect();
            if tokens.is_empty() {
                bail!("none of the tokens were found on the scene");
            }
            let data: Vec<Value> = tokens
                .iter()
                .map(|t| {
                    json!({
                        "tokenId": t["_id"], "sceneId": scene["_id"], "actorId": t["actorId"],
                        "hidden": t.get("hidden").cloned().unwrap_or(json!(false)),
                    })
                })
                .collect();
            let result = foundry.modify_document("Combatant", "create", json!({
                "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                "parentUuid": format!("Combat.{combat_id}"), "data": data,
            })).await?;
            Ok(text_response(&json!({
                "combat": combat_id,
                "added": tokens.iter().map(|t| t["name"].clone()).collect::<Vec<_>>(),
                "result": result,
            })))
        }
        "set_initiative" => {
            let who =
                str_arg(args, "combatant").ok_or_else(|| anyhow!("'combatant' is required"))?;
            let initiative = args
                .get("initiative")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("'initiative' is required"))?;
            let empty = vec![];
            let all = combat
                .get("combatants")
                .and_then(Value::as_array)
                .unwrap_or(&empty);
            let target = all
                .iter()
                .find(|c| c["_id"] == json!(who) || c["name"] == json!(who))
                .ok_or_else(|| anyhow!("Combatant not found: {who}"))?;
            let result = foundry
                .modify_document(
                    "Combatant",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "parentUuid": format!("Combat.{combat_id}"),
                        "updates": [{"_id": target["_id"], "initiative": initiative}],
                    }),
                )
                .await?;
            Ok(text_response(&json!({
                "combat": combat_id, "combatant": target["name"], "initiative": initiative, "result": result,
            })))
        }
        "start" => {
            let result = foundry
                .modify_document(
                    "Combat",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{"_id": combat_id, "round": 1, "turn": 0}],
                    }),
                )
                .await?;
            Ok(text_response(
                &json!({"combat": combat_id, "started": true, "result": result}),
            ))
        }
        "next_turn" | "next_round" => {
            let order = with_names(state, &combat, sorted(&combat)).await?;
            let round = combat.get("round").and_then(Value::as_u64).unwrap_or(0);
            let turn = combat.get("turn").and_then(Value::as_u64).unwrap_or(0);
            let (new_round, new_turn) =
                if action == "next_round" || (turn + 1) as usize >= order.len() {
                    (round + 1, 0u64)
                } else {
                    (round, turn + 1)
                };
            let result = foundry
                .modify_document(
                    "Combat",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{"_id": combat_id, "round": new_round, "turn": new_turn}],
                    }),
                )
                .await?;
            Ok(text_response(&json!({
                "combat": combat_id, "round": new_round, "turn": new_turn,
                "current": order.get(new_turn as usize).and_then(|c| c.get("name")).cloned().unwrap_or(Value::Null),
                "result": result,
            })))
        }
        "status" => {
            let order = with_names(state, &combat, sorted(&combat)).await?;
            let turn = combat.get("turn").and_then(Value::as_u64).unwrap_or(0) as usize;
            Ok(text_response(&json!({
                "combat": combat_id, "active": combat["active"],
                "round": combat["round"], "turn": combat["turn"],
                "current": order.get(turn).and_then(|c| c.get("name")).cloned().unwrap_or(Value::Null),
                "order": order.iter().map(|c| json!({
                    "_id": c["_id"], "name": c["name"], "initiative": c["initiative"], "defeated": c["defeated"],
                })).collect::<Vec<_>>(),
            })))
        }
        "end" => {
            let result = foundry
                .modify_document(
                    "Combat",
                    "delete",
                    json!({
                        "action": "delete", "broadcast": false, "ids": [combat_id],
                    }),
                )
                .await?;
            Ok(text_response(
                &json!({"combat": combat_id, "ended": true, "result": result}),
            ))
        }
        other => bail!("Unknown action '{other}'"),
    }
}

async fn control_playlist(state: &McpState, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    let pl_arg = str_arg(args, "playlist").ok_or_else(|| anyhow!("'playlist' is required"))?;
    let action = str_arg(args, "action").ok_or_else(|| anyhow!("'action' is required"))?;
    let playlist = foundry
        .find_document("playlists", &pl_arg, None)
        .await?
        .ok_or_else(|| anyhow!("Playlist not found: {pl_arg}"))?;
    let mut sounds = playlist
        .get("sounds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    sounds.sort_by_key(|s| s.get("sort").and_then(Value::as_i64).unwrap_or(0));
    let mode = playlist.get("mode").and_then(Value::as_i64).unwrap_or(0);

    let (playing, update) = if action == "stop" {
        (
            vec![],
            json!({
                "playing": false,
                "sounds": sounds.iter().map(|s| json!({"_id": s["_id"], "playing": false})).collect::<Vec<_>>(),
            }),
        )
    } else {
        let playing_ids: Vec<Value> = match str_arg(args, "sound") {
            Some(sarg) => {
                let s = sounds
                    .iter()
                    .find(|s| s["_id"] == json!(sarg) || s["name"] == json!(sarg))
                    .ok_or_else(|| anyhow!("Sound not found in playlist: {sarg}"))?;
                vec![s["_id"].clone()]
            }
            None if mode == 2 => sounds.iter().map(|s| s["_id"].clone()).collect(),
            None if mode == -1 => bail!("soundboard playlist — provide 'sound'"),
            None => {
                let first = sounds
                    .first()
                    .ok_or_else(|| anyhow!("playlist has no sounds"))?;
                vec![first["_id"].clone()]
            }
        };
        let names: Vec<Value> = sounds
            .iter()
            .filter(|s| playing_ids.contains(&s["_id"]))
            .map(|s| s["name"].clone())
            .collect();
        (
            names,
            json!({
                "playing": true,
                "sounds": sounds.iter().map(|s| json!({
                    "_id": s["_id"], "playing": playing_ids.contains(&s["_id"]),
                })).collect::<Vec<_>>(),
            }),
        )
    };
    let mut update_doc = update;
    update_doc["_id"] = playlist["_id"].clone();
    let result = foundry
        .modify_document(
            "Playlist",
            "update",
            json!({
                "action": "update", "diff": false, "recursive": true, "render": true,
                "updates": [update_doc],
            }),
        )
        .await?;
    Ok(text_response(&json!({
        "playlist": {"_id": playlist["_id"], "name": playlist["name"]},
        "action": action, "playing": playing, "result": result,
    })))
}

async fn draw_from_table(state: &McpState, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    let t_arg = str_arg(args, "table").ok_or_else(|| anyhow!("'table' is required"))?;
    let table = foundry
        .find_document("tables", &t_arg, None)
        .await?
        .ok_or_else(|| anyhow!("RollTable not found: {t_arg}"))?;
    let modifier = args.get("modifier").and_then(Value::as_i64).unwrap_or(0);
    let rolls_n = args
        .get("rolls")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 10);
    let draws = super::roll_table_draws(&table, modifier, rolls_n as usize)?;

    let mut posted = false;
    if args.get("post").and_then(Value::as_bool).unwrap_or(true) {
        let body: Vec<String> = draws
            .iter()
            .map(|d| {
                format!(
                    "<p><strong>{}</strong>{} → {}</p>",
                    d["roll"],
                    if modifier != 0 {
                        format!(" (dont {modifier:+})")
                    } else {
                        String::new()
                    },
                    d["text"].as_str().unwrap_or("<em>hors table</em>"),
                )
            })
            .collect();
        let mut message = json!({
            "content": format!("<h3>🎲 {}</h3>\n{}", table["name"].as_str().unwrap_or("Table"), body.join("\n")),
            "author": foundry.user_id().await,
            "flags": {"foundry-mcp": {"tableDraw": {"table": table["_id"], "draws": draws}}},
        });
        if let Some(w) = args.get("whisper_users").and_then(Value::as_array)
            && !w.is_empty()
        {
            message["whisper"] = json!(w);
        }
        foundry
            .modify_document(
                "ChatMessage",
                "create",
                json!({
                    "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                    "data": [message],
                }),
            )
            .await?;
        posted = true;
    }
    Ok(text_response(&json!({
        "table": {"_id": table["_id"], "name": table["name"]},
        "modifier": modifier, "draws": draws, "posted": posted,
    })))
}
