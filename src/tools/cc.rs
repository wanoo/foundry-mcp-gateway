//! Campaign Codex : fiches = JournalEntry + flags["campaign-codex"]
//! {type, image?, data} — schéma relevé sur un monde réel (v13).

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::{str_arg, text_response};
use crate::foundry::documents::get_path;
use crate::mcp::McpState;

pub const CC_TYPES: [&str; 7] = ["npc", "group", "location", "region", "shop", "quest", "tag"];

fn data_defaults(cc_type: &str) -> Value {
    match cc_type {
        "npc" => json!({"linkedActor": null, "description": "", "linkedLocations": [], "linkedShops": [], "associates": [], "notes": "", "tagMode": false}),
        "group" => json!({"description": "", "associates": []}),
        "location" => json!({"description": "", "tags": [], "widgets": {}}),
        "region" => json!({"description": "", "tags": []}),
        "shop" => json!({"description": "", "linkedNPCs": [], "linkedLocation": null, "inventory": [], "linkedScene": null, "markup": 1, "notes": "", "inventoryCacheVersion": 1}),
        "quest" => json!({"description": "", "associates": [], "notes": ""}),
        "tag" => json!({"linkedActor": null, "description": "", "linkedLocations": [], "linkedShops": [], "associates": [], "notes": "", "tagMode": true}),
        _ => json!({}),
    }
}

pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("cc_list_sheets",
         "List Campaign Codex sheets (type/tag/name filters), light index with link counts.",
         json!({"type":"object","properties":{
            "type":{"type":"string","enum":CC_TYPES},
            "tag":{"type":"string"},"name_contains":{"type":"string"}}})),
        ("cc_get_sheet",
         "One Campaign Codex sheet: type, image, full data (links, tags, notes).",
         json!({"type":"object","properties":{"_id":{"type":"string"},"name":{"type":"string"}}})),
        ("cc_create_sheet",
         "Create a Campaign Codex sheet with the correct flag structure for its type.",
         json!({"type":"object","properties":{
            "name":{"type":"string"},"type":{"type":"string","enum":CC_TYPES},
            "description":{"type":"string"},"image":{"type":"string"},
            "linked_actor":{"type":"string"},
            "tags":{"type":"array","items":{"type":"string"}},
            "gm_only":{"type":"boolean"}},"required":["name","type"]})),
        ("cc_link",
         "Link two CC sheets (or a sheet to an actor). Relations: associates (default), linkedLocations, linkedShops, linkedNPCs, linkedLocation, linkedActor. bidirectional adds the reverse associate.",
         json!({"type":"object","properties":{
            "from":{"type":"string"},"to":{"type":"string"},
            "relation":{"type":"string","enum":["associates","linkedLocations","linkedShops","linkedNPCs","linkedLocation","linkedActor"]},
            "bidirectional":{"type":"boolean"}},"required":["from","to"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

async fn get_sheet(state: &McpState, ident: &str) -> Result<Option<Value>> {
    let doc = state.foundry.find_document("journal", ident, None).await?;
    Ok(doc.filter(|d| get_path(d, "flags.campaign-codex").is_some()))
}

fn cc_of(doc: &Value) -> &Value {
    get_path(doc, "flags.campaign-codex").unwrap_or(&Value::Null)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "cc_list_sheets" => {
            let mut where_ = serde_json::Map::new();
            where_.insert("flags.campaign-codex__exists".into(), json!(true));
            if let Some(t) = str_arg(args, "type") {
                where_.insert("flags.campaign-codex.type".into(), json!(t));
            }
            if let Some(n) = str_arg(args, "name_contains") {
                where_.insert("name__contains".into(), json!(n));
            }
            if let Some(tag) = str_arg(args, "tag") {
                where_.insert("flags.campaign-codex.data.tags__contains".into(), json!(tag));
            }
            let sheets = foundry.get_documents("journal", Some(&where_), None, 0, None).await?;
            let index: Vec<Value> = sheets.iter().map(|s| {
                let cc = cc_of(s);
                let data = cc.get("data").cloned().unwrap_or(json!({}));
                let count = |k: &str| data.get(k).and_then(Value::as_array).map(Vec::len).unwrap_or(0);
                json!({
                    "_id": s["_id"], "name": s["name"], "type": cc["type"],
                    "tags": data.get("tags").cloned(),
                    "linkedActor": data.get("linkedActor").cloned(),
                    "links": count("associates") + count("linkedLocations") + count("linkedShops") + count("linkedNPCs"),
                })
            }).collect();
            Ok(text_response(&Value::Array(index)))
        }
        "cc_get_sheet" => {
            let ident = str_arg(args, "_id").or_else(|| str_arg(args, "name"))
                .ok_or_else(|| anyhow!("Must provide one of: _id or name"))?;
            let sheet = get_sheet(state, &ident).await?
                .ok_or_else(|| anyhow!("No Campaign Codex sheet found with that identifier"))?;
            let cc = cc_of(&sheet);
            Ok(text_response(&json!({
                "_id": sheet["_id"], "name": sheet["name"],
                "type": cc["type"], "image": cc.get("image").cloned(),
                "data": cc.get("data").cloned().unwrap_or(json!({})),
                "ownership": sheet.get("ownership").cloned(),
            })))
        }
        "cc_create_sheet" => {
            let sheet_name = str_arg(args, "name").ok_or_else(|| anyhow!("'name' is required"))?;
            let cc_type = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            if !CC_TYPES.contains(&cc_type.as_str()) {
                bail!("Unknown Campaign Codex type '{cc_type}'. Valid: {}", CC_TYPES.join(", "));
            }
            let description = str_arg(args, "description").unwrap_or_default();
            let mut data = data_defaults(&cc_type);
            data["description"] = json!(description);
            if let (Some(tags), Some(_)) = (args.get("tags"), data.get("tags")) {
                data["tags"] = tags.clone();
            }
            if let Some(actor_arg) = str_arg(args, "linked_actor") {
                if data.get("linkedActor").is_some() {
                    let fields = vec!["_id".into(), "name".into()];
                    let actor = foundry.find_document("actors", &actor_arg, Some(&fields)).await?
                        .ok_or_else(|| anyhow!("linked_actor not found: {actor_arg}"))?;
                    data["linkedActor"] = json!(format!("Actor.{}", actor["_id"].as_str().unwrap_or("")));
                }
            }
            let mut flags_cc = json!({"type": cc_type, "data": data});
            if let Some(img) = str_arg(args, "image") { flags_cc["image"] = json!(img); }
            let gm_only = args.get("gm_only").and_then(Value::as_bool).unwrap_or(false);
            let doc = json!({
                "name": sheet_name,
                "flags": {"campaign-codex": flags_cc},
                "pages": [{"name": sheet_name, "type": "text", "text": {"content": description}}],
                "ownership": {"default": if gm_only { 0 } else { 2 }},
            });
            let result = foundry.modify_document("JournalEntry", "create", json!({
                "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                "data": [doc],
            })).await?;
            let created_id = result.pointer("/result/0/_id").cloned().unwrap_or(Value::Null);
            Ok(text_response(&json!({
                "created": {"_id": created_id, "name": sheet_name, "type": cc_type},
                "result": result,
            })))
        }
        "cc_link" => {
            let from_arg = str_arg(args, "from").ok_or_else(|| anyhow!("'from' is required"))?;
            let to_arg = str_arg(args, "to").ok_or_else(|| anyhow!("'to' is required"))?;
            let relation = str_arg(args, "relation").unwrap_or_else(|| "associates".into());
            let from = get_sheet(state, &from_arg).await?
                .ok_or_else(|| anyhow!("Source sheet not found: {from_arg}"))?;
            let from_data = get_path(&from, "flags.campaign-codex.data").cloned().unwrap_or(json!({}));

            let (target_ref, target_label) = if relation == "linkedActor" {
                let fields = vec!["_id".into(), "name".into()];
                let actor = foundry.find_document("actors", &to_arg, Some(&fields)).await?
                    .ok_or_else(|| anyhow!("Actor not found: {to_arg}"))?;
                (format!("Actor.{}", actor["_id"].as_str().unwrap_or("")),
                 actor["name"].as_str().unwrap_or("").to_string())
            } else {
                let to = get_sheet(state, &to_arg).await?
                    .ok_or_else(|| anyhow!("Target sheet not found: {to_arg}"))?;
                if args.get("bidirectional").and_then(Value::as_bool).unwrap_or(false) {
                    let to_assoc = get_path(&to, "flags.campaign-codex.data.associates")
                        .and_then(Value::as_array).cloned().unwrap_or_default();
                    let from_ref = json!(format!("JournalEntry.{}", from["_id"].as_str().unwrap_or("")));
                    if !to_assoc.contains(&from_ref) {
                        let mut list = to_assoc;
                        list.push(from_ref);
                        foundry.modify_document("JournalEntry", "update", json!({
                            "action": "update", "diff": false, "recursive": true, "render": true,
                            "updates": [{"_id": to["_id"],
                                "flags": {"campaign-codex": {"data": {"associates": list}}}}],
                        })).await?;
                    }
                }
                (format!("JournalEntry.{}", to["_id"].as_str().unwrap_or("")),
                 to["name"].as_str().unwrap_or("").to_string())
            };

            let update = if relation == "linkedActor" || relation == "linkedLocation" {
                json!({relation.clone(): target_ref})
            } else {
                let list = from_data.get(&relation).and_then(Value::as_array).cloned().unwrap_or_default();
                if list.contains(&json!(target_ref)) {
                    return Ok(text_response(&json!({
                        "from": from["name"], "to": target_label,
                        "relation": relation, "unchanged": "already linked",
                    })));
                }
                let mut list = list;
                list.push(json!(target_ref));
                json!({relation.clone(): list})
            };
            let result = foundry.modify_document("JournalEntry", "update", json!({
                "action": "update", "diff": false, "recursive": true, "render": true,
                "updates": [{"_id": from["_id"], "flags": {"campaign-codex": {"data": update}}}],
            })).await?;
            Ok(text_response(&json!({
                "from": from["name"], "to": target_label, "relation": relation,
                "bidirectional": args.get("bidirectional").and_then(Value::as_bool).unwrap_or(false),
                "result": result,
            })))
        }
        other => bail!("Unknown tool: {other}"),
    }
}
