//! Module starwarsffg : 7 outils (noms historiques sans préfixe).
//! Chemins vérifiés sur starwarsffg 2.0.3 (fiches réelles).

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::swffg_derived::derive_skill_pool;
use super::swffg_dice::{format_pool, format_result, result_json, roll_ffg_pool, FfgPool};
use crate::mcp::McpState;
use crate::tools::{post_chat, roll_table_draws, str_arg, text_response};

const STAT_PATHS: [(&str, &str); 11] = [
    ("wounds", "system.stats.wounds.value"),
    ("strain", "system.stats.strain.value"),
    ("credits", "system.stats.credits.value"),
    ("xp_available", "system.experience.available"),
    ("xp_total", "system.experience.total"),
    ("obligation", "system.obligation.value"),
    ("duty", "system.duty.value"),
    ("morality", "system.morality.value"),
    ("conflict", "system.conflict.value"),
    ("hull_trauma", "system.stats.hullTrauma.value"),
    ("system_strain", "system.stats.systemStrain.value"),
];

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    let dice_props = json!({
        "ability":{"type":"number"},"proficiency":{"type":"number"},
        "difficulty":{"type":"number"},"challenge":{"type":"number"},
        "boost":{"type":"number"},"setback":{"type":"number"},"force":{"type":"number"}});
    vec![
        ("request_player_roll",
         "Post an FFG roll request in chat: '🎲' button opening the dice-pool dialog pre-filled for the player (ffg-pool-to-player). starwarsffg only.",
         json!({"type":"object","properties":{
            "description":{"type":"string"},"content":{"type":"string"},
            "skill_name":{"type":"string"},
            "whisper_users":{"type":"array","items":{"type":"string"}},
            "difficulty":{"type":"number"},"challenge":{"type":"number"},
            "ability":{"type":"number"},"proficiency":{"type":"number"},
            "boost":{"type":"number"},"setback":{"type":"number"},"force":{"type":"number"}},
            "required":["description"]})),
        ("roll_ffg_pool",
         "Roll an FFG narrative dice pool SERVER-SIDE (official faces) and post to chat — no GM browser needed.",
         json!({"type":"object","properties": {
            "description":{"type":"string"},"post":{"type":"boolean"},
            "whisper_users":{"type":"array","items":{"type":"string"}},
            "ability":{"type":"number"},"proficiency":{"type":"number"},
            "difficulty":{"type":"number"},"challenge":{"type":"number"},
            "boost":{"type":"number"},"setback":{"type":"number"},"force":{"type":"number"}},
            "required":["description"]})),
        ("adjust_actor_stats",
         "Adjust starwarsffg stats without knowing paths: wounds, strain, credits, xp_available, xp_total, obligation, duty, morality, conflict; vehicles: hull_trauma, system_strain. DELTAS by default; set:true = absolute. Floor 0.",
         json!({"type":"object","properties":{
            "actor":{"type":"string"},"set":{"type":"boolean"},
            "wounds":{"type":"number"},"strain":{"type":"number"},"credits":{"type":"number"},
            "xp_available":{"type":"number"},"xp_total":{"type":"number"},
            "obligation":{"type":"number"},"duty":{"type":"number"},
            "morality":{"type":"number"},"conflict":{"type":"number"},
            "hull_trauma":{"type":"number"},"system_strain":{"type":"number"}},
            "required":["actor"]})),
        ("roll_actor_skill",
         "Roll a skill FOR an actor: the REAL pool is derived from the sheet (stored + species/equipment/learned-talent mods — the source-doc-at-0 trap is handled), plus your difficulty. Posts derivation detail.",
         {
            let mut schema = json!({"type":"object","properties":{
                "actor":{"type":"string"},"skill":{"type":"string"},
                "post":{"type":"boolean"},
                "whisper_users":{"type":"array","items":{"type":"string"}}},
                "required":["actor","skill"]});
            for (k, v) in dice_props.as_object().unwrap() {
                schema["properties"][k] = v.clone();
            }
            schema
         }),
        ("adjust_destiny",
         "Destiny Pool (starwarsffg.dPoolLight/Dark settings): read, spend_light/spend_dark (converts to the other side), set.",
         json!({"type":"object","properties":{
            "action":{"type":"string","enum":["read","spend_light","spend_dark","set"]},
            "light":{"type":"number"},"dark":{"type":"number"}},"required":["action"]})),
        ("grant_xp",
         "Grant XP to actors (available AND total). Default targets: every character-type actor.",
         json!({"type":"object","properties":{
            "amount":{"type":"number"},
            "actors":{"type":"array","items":{"type":"string"}}},"required":["amount"]})),
        ("apply_critical_injury",
         "By-the-book critical injury: +10/existing injury on the d100, draws the crit table, attaches the linked compendium item to the actor, posts to chat.",
         json!({"type":"object","properties":{
            "actor":{"type":"string"},"table":{"type":"string"},
            "extra_modifier":{"type":"number"},"roll_only":{"type":"boolean"}},
            "required":["actor"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

fn get_path_num(doc: &Value, dotted: &str) -> i64 {
    crate::foundry::documents::get_path(doc, dotted)
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "request_player_roll" => {
            let description =
                str_arg(args, "description").ok_or_else(|| anyhow!("'description' is required"))?;
            let mut pool = serde_json::Map::new();
            for die in [
                "difficulty",
                "challenge",
                "ability",
                "proficiency",
                "boost",
                "setback",
                "force",
            ] {
                if let Some(n) = args.get(die).and_then(Value::as_u64).filter(|n| *n > 0) {
                    pool.insert(die.into(), json!(n));
                }
            }
            let body =
                str_arg(args, "content").unwrap_or_else(|| format!("<h3>🎲 {description}</h3>"));
            let content = format!(
                "{body}\n<button class=\"ffg-pool-to-player\">🎲 Lancer — {description}</button>"
            );
            let flags = json!({"starwarsffg": {
                "dicePool": pool, "description": description,
                "roll": {"data": {}, "skillName": str_arg(args, "skill_name").unwrap_or_else(|| description.clone()),
                          "item": {}, "flavor": "", "sound": null},
            }});
            let result = post_chat(state, &content, flags, args.get("whisper_users")).await?;
            Ok(text_response(
                &json!({"posted": description, "pool": pool, "result": result}),
            ))
        }
        "roll_ffg_pool" => {
            let description =
                str_arg(args, "description").ok_or_else(|| anyhow!("'description' is required"))?;
            let pool = FfgPool::from_args(args);
            let roll = {
                let mut rng = rand::rng();
                roll_ffg_pool(&pool, || rand::Rng::random::<f64>(&mut rng))
            };
            let summary = format_result(&roll);
            let mut posted = false;
            if args.get("post").and_then(Value::as_bool).unwrap_or(true) {
                let content = format!(
                    "<h3>🎲 {description}</h3><p>{}</p><p><strong>{summary}</strong></p>",
                    format_pool(&pool)
                );
                post_chat(
                    state,
                    &content,
                    json!({"foundry-mcp": {"roll": {"result": result_json(&roll)}}}),
                    args.get("whisper_users"),
                )
                .await?;
                posted = true;
            }
            Ok(text_response(&json!({
                "description": description, "pool": format_pool(&pool),
                "summary": summary, "detail": result_json(&roll), "posted": posted,
            })))
        }
        "adjust_actor_stats" => {
            let actor_arg = str_arg(args, "actor").ok_or_else(|| anyhow!("'actor' is required"))?;
            let requested: Vec<&(&str, &str)> = STAT_PATHS
                .iter()
                .filter(|(k, _)| args.get(*k).is_some())
                .collect();
            if requested.is_empty() {
                bail!(
                    "provide at least one of: {}",
                    STAT_PATHS
                        .iter()
                        .map(|(k, _)| *k)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            let fields = vec!["_id".into(), "name".into(), "system".into()];
            let actor = foundry
                .find_document("actors", &actor_arg, Some(&fields))
                .await?
                .ok_or_else(|| anyhow!("Actor not found: {actor_arg}"))?;
            let absolute = args.get("set").and_then(Value::as_bool).unwrap_or(false);
            let mut update = serde_json::Map::new();
            let mut changes = serde_json::Map::new();
            for (key, path) in requested {
                let before = get_path_num(&actor, path);
                let input = args.get(*key).and_then(Value::as_i64).unwrap_or(0);
                let after = (if absolute { input } else { before + input }).max(0);
                update.insert((*path).into(), json!(after));
                changes.insert((*key).into(), json!({"before": before, "after": after}));
            }
            let mut update_doc = Value::Object(update);
            update_doc["_id"] = actor["_id"].clone();
            let result = foundry
                .modify_document(
                    "Actor",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [update_doc],
                    }),
                )
                .await?;
            Ok(text_response(&json!({
                "actor": {"_id": actor["_id"], "name": actor["name"]},
                "mode": if absolute { "set" } else { "delta" },
                "changes": changes, "result": result,
            })))
        }
        "roll_actor_skill" => {
            let actor_arg = str_arg(args, "actor").ok_or_else(|| anyhow!("'actor' is required"))?;
            let skill = str_arg(args, "skill").ok_or_else(|| anyhow!("'skill' is required"))?;
            let actor = foundry
                .find_document("actors", &actor_arg, None)
                .await?
                .ok_or_else(|| anyhow!("Actor not found: {actor_arg}"))?;
            let derived = derive_skill_pool(&actor, &skill)
                .ok_or_else(|| anyhow!("Skill '{skill}' not found on {}", actor["name"]))?;
            let extra = |k: &str| args.get(k).and_then(Value::as_i64).unwrap_or(0);
            let setback = (extra("setback") + derived["setback"].as_i64().unwrap_or(0)
                - derived["removeSetback"].as_i64().unwrap_or(0))
            .max(0) as u32;
            let pool = FfgPool {
                ability: derived["ability"].as_u64().unwrap_or(0) as u32,
                proficiency: derived["proficiency"].as_u64().unwrap_or(0) as u32,
                boost: derived["boost"].as_u64().unwrap_or(0) as u32 + extra("boost").max(0) as u32,
                setback,
                difficulty: extra("difficulty").max(0) as u32,
                challenge: extra("challenge").max(0) as u32,
                force: extra("force").max(0) as u32,
            };
            let roll = {
                let mut rng = rand::rng();
                roll_ffg_pool(&pool, || rand::Rng::random::<f64>(&mut rng))
            };
            let summary = format_result(&roll);
            let derivation = format!(
                "{} {} + rang {}{}",
                derived["characteristic"].as_str().unwrap_or("?"),
                derived["characteristicValue"],
                derived["rank"],
                {
                    let s = derived["sources"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    if s.is_empty() {
                        String::new()
                    } else {
                        format!(" · mods : {s}")
                    }
                }
            );
            let mut posted = false;
            if args.get("post").and_then(Value::as_bool).unwrap_or(true) {
                let content = format!(
                    "<h3>🎲 {} — {}</h3><p>{}</p><p><strong>{summary}</strong></p><p style=\"font-size:.85em\">{derivation}</p>",
                    actor["name"].as_str().unwrap_or("?"),
                    derived["skill"].as_str().unwrap_or(&skill),
                    format_pool(&pool)
                );
                post_chat(state, &content,
                    json!({"foundry-mcp": {"skillRoll": {"actor": actor["_id"], "result": result_json(&roll)}}}),
                    args.get("whisper_users")).await?;
                posted = true;
            }
            Ok(text_response(&json!({
                "actor": {"_id": actor["_id"], "name": actor["name"]},
                "skill": derived["skill"], "derivation": derived,
                "pool": format_pool(&pool), "summary": summary,
                "detail": result_json(&roll), "posted": posted,
            })))
        }
        "adjust_destiny" => {
            let action = str_arg(args, "action").ok_or_else(|| anyhow!("'action' is required"))?;
            let keys = ["starwarsffg.dPoolLight", "starwarsffg.dPoolDark"];
            let w = json!({"key__in": keys}).as_object().cloned().unwrap();
            let docs = foundry
                .get_documents("settings", Some(&w), None, 0, None)
                .await?;
            let read = |key: &str| -> (Option<Value>, i64) {
                let doc = docs.iter().find(|d| d["key"] == json!(key));
                let value = doc
                    .and_then(|d| d.get("value"))
                    .and_then(Value::as_str)
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                (doc.and_then(|d| d.get("_id")).cloned(), value)
            };
            let (light_id, mut light) = read(keys[0]);
            let (dark_id, mut dark) = read(keys[1]);
            let before = json!({"light": light, "dark": dark});
            match action.as_str() {
                "read" => return Ok(text_response(&before)),
                "spend_light" => {
                    if light <= 0 {
                        bail!("no light-side destiny point to spend");
                    }
                    light -= 1;
                    dark += 1;
                }
                "spend_dark" => {
                    if dark <= 0 {
                        bail!("no dark-side destiny point to spend");
                    }
                    dark -= 1;
                    light += 1;
                }
                "set" => {
                    if args.get("light").is_none() && args.get("dark").is_none() {
                        bail!("set requires light and/or dark");
                    }
                    if let Some(l) = args.get("light").and_then(Value::as_i64) {
                        light = l.max(0);
                    }
                    if let Some(d) = args.get("dark").and_then(Value::as_i64) {
                        dark = d.max(0);
                    }
                }
                other => bail!("Unknown action '{other}'"),
            }
            for (key, id, value, old) in [
                (keys[0], light_id, light, before["light"].as_i64().unwrap()),
                (keys[1], dark_id, dark, before["dark"].as_i64().unwrap()),
            ] {
                if value == old {
                    continue;
                }
                match id {
                    Some(doc_id) => {
                        foundry.modify_document("Setting", "update", json!({
                            "action": "update", "diff": false, "recursive": true, "render": true,
                            "updates": [{"_id": doc_id, "value": value.to_string()}],
                        })).await?;
                    }
                    None => {
                        foundry.modify_document("Setting", "create", json!({
                            "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                            "data": [{"key": key, "value": value.to_string()}],
                        })).await?;
                    }
                }
            }
            Ok(text_response(
                &json!({"action": action, "light": light, "dark": dark, "before": before}),
            ))
        }
        "grant_xp" => {
            let amount = args
                .get("amount")
                .and_then(Value::as_i64)
                .filter(|a| *a != 0)
                .ok_or_else(|| anyhow!("'amount' is required"))?;
            let fields = vec!["_id".into(), "name".into(), "system".into()];
            let targets: Vec<Value> = match args.get("actors").and_then(Value::as_array) {
                Some(wanted) if !wanted.is_empty() => {
                    let mut list = Vec::new();
                    for w in wanted.iter().filter_map(Value::as_str) {
                        let a = foundry
                            .find_document("actors", w, Some(&fields))
                            .await?
                            .ok_or_else(|| anyhow!("Actor not found: {w}"))?;
                        list.push(a);
                    }
                    list
                }
                _ => {
                    let w = json!({"type": "character"}).as_object().cloned().unwrap();
                    foundry
                        .get_documents("actors", Some(&w), Some(&fields), 0, None)
                        .await?
                }
            };
            if targets.is_empty() {
                bail!("no target actors");
            }
            let mut granted = Vec::new();
            for actor in &targets {
                let available = get_path_num(actor, "system.experience.available") + amount;
                let total = get_path_num(actor, "system.experience.total") + amount;
                foundry
                    .modify_document(
                        "Actor",
                        "update",
                        json!({
                            "action": "update", "diff": false, "recursive": true, "render": true,
                            "updates": [{"_id": actor["_id"],
                                "system.experience.available": available,
                                "system.experience.total": total}],
                        }),
                    )
                    .await?;
                granted.push(json!({"_id": actor["_id"], "name": actor["name"], "available": available, "total": total}));
            }
            Ok(text_response(
                &json!({"amount": amount, "granted": granted}),
            ))
        }
        "apply_critical_injury" => {
            let actor_arg = str_arg(args, "actor").ok_or_else(|| anyhow!("'actor' is required"))?;
            let fields = vec!["_id".into(), "name".into(), "items".into()];
            let actor = foundry
                .find_document("actors", &actor_arg, Some(&fields))
                .await?
                .ok_or_else(|| anyhow!("Actor not found: {actor_arg}"))?;
            let empty = vec![];
            let existing = actor
                .get("items")
                .and_then(Value::as_array)
                .unwrap_or(&empty)
                .iter()
                .filter(|i| i["type"] == json!("criticalinjury"))
                .count();
            let modifier = existing as i64 * 10
                + args
                    .get("extra_modifier")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);

            let table = match str_arg(args, "table") {
                Some(t) => foundry.find_document("tables", &t, None).await?,
                None => {
                    let tables = foundry.get_documents("tables", None, None, 0, None).await?;
                    tables.into_iter().find(|t| {
                        let n = t["name"].as_str().unwrap_or("").to_lowercase();
                        n.contains("crit") && (n.contains("blessure") || n.contains("injur"))
                    })
                }
            }
            .ok_or_else(|| anyhow!("critical-injury RollTable not found (pass 'table')"))?;

            let draws = roll_table_draws(&table, modifier, 1)?;
            let draw = &draws[0];
            let text = draw["text"].as_str().unwrap_or("").to_string();

            let mut attached = Value::Null;
            if !args
                .get("roll_only")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && !text.is_empty()
            {
                let re = regex::Regex::new(
                    r"@UUID\[Compendium\.([A-Za-z0-9_-]+\.[A-Za-z0-9_-]+)\.(?:Item\.)?([A-Za-z0-9]+)\]"
                ).unwrap();
                if let Some(caps) = re.captures(&text) {
                    let pack = &caps[1];
                    let item_id = &caps[2];
                    let mut query = serde_json::Map::new();
                    query.insert("_id".into(), json!(item_id));
                    let docs = foundry
                        .get_collection("Item", query, false, Some(pack))
                        .await?;
                    if let Some(mut injury) = docs.into_iter().next() {
                        if let Some(obj) = injury.as_object_mut() {
                            obj.remove("folder");
                        }
                        attached = json!({"name": injury["name"]});
                        foundry.modify_document("Item", "create", json!({
                            "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                            "parentUuid": format!("Actor.{}", actor["_id"].as_str().unwrap_or("")),
                            "data": [injury],
                        })).await?;
                    }
                }
            }
            let content = format!(
                "<h3>🩸 Blessure critique — {}</h3><p>Tirage : <strong>{}</strong>{}</p><blockquote>{}</blockquote>",
                actor["name"].as_str().unwrap_or("?"), draw["roll"],
                if modifier != 0 { format!(" (dont +{modifier} : {existing} blessure(s) existante(s))") } else { String::new() },
                if text.is_empty() { "<em>hors table</em>" } else { &text },
            );
            post_chat(state, &content,
                json!({"foundry-mcp": {"criticalInjury": {"actor": actor["_id"], "roll": draw["roll"], "modifier": modifier}}}),
                None).await?;
            Ok(text_response(&json!({
                "actor": {"_id": actor["_id"], "name": actor["name"]},
                "roll": draw["roll"], "modifier": modifier,
                "existing_injuries": existing, "result": text, "attached": attached,
            })))
        }
        other => bail!("Unknown tool: {other}"),
    }
}
