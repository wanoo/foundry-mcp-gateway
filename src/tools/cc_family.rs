//! Famille d'addons wgtnGM (« famille Campaign Codex ») — outils MCP + délégations
//! au module compagnon, regroupés :
//!   · Campaign Codex   (cc_*)     fiches = JournalEntry + flags["campaign-codex"]
//!   · Asset Librarian  (al_*)     tags flags["asset-librarian"] + navigateur (client)
//!   · Mini Calendar    (mc_*)     temps (core.time) + notes (journaux) + client
//! Les outils `client_*` de cette famille délèguent au compagnon (API client-side
//! game.assetLibrarian / game.time / macros du calendrier).

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::companion::call_companion;
use super::{str_arg, text_response};
use crate::foundry::documents::get_path;
use crate::mcp::McpState;

pub const CC_TYPES: [&str; 7] = ["npc", "group", "location", "region", "shop", "quest", "tag"];

/// Journaux où Mini Calendar range ses notes (relevés sur un monde réel).
const MC_NOTE_JOURNALS: [(&str, &str); 2] = [
    ("player", "Player Notes - Mini Calendar"),
    ("events", "Calendar Events - Mini Calendar"),
];

fn data_defaults(cc_type: &str) -> Value {
    match cc_type {
        "npc" => {
            json!({"linkedActor": null, "description": "", "linkedLocations": [], "linkedShops": [], "associates": [], "notes": "", "tagMode": false})
        }
        "group" => json!({"description": "", "associates": []}),
        "location" => json!({"description": "", "tags": [], "widgets": {}}),
        "region" => json!({"description": "", "tags": []}),
        "shop" => {
            json!({"description": "", "linkedNPCs": [], "linkedLocation": null, "inventory": [], "linkedScene": null, "markup": 1, "notes": "", "inventoryCacheVersion": 1})
        }
        "quest" => json!({"description": "", "associates": [], "notes": ""}),
        "tag" => {
            json!({"linkedActor": null, "description": "", "linkedLocations": [], "linkedShops": [], "associates": [], "notes": "", "tagMode": true})
        }
        _ => json!({}),
    }
}

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
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
        // --- Campaign Codex client-side (compagnon) ---
        ("client_cc_convert",
         "Campaign Codex (companion): convert a Journal Entry (uuid) into a CC sheet of the given type — great for bulk migration.",
         json!({"type":"object","properties":{
            "uuid":{"type":"string"},"type":{"type":"string"},
            "pages_to_separate_sheets":{"type":"boolean"}},"required":["uuid","type"]})),
        ("client_cc_export_obsidian",
         "Campaign Codex (companion): export the whole codex to a Markdown/Obsidian zip.",
         json!({"type":"object","properties":{}})),
        ("client_cc_open_toc",
         "Campaign Codex (companion): open the Table of Contents on the GM client (optional tab).",
         json!({"type":"object","properties":{"tab":{"type":"string"}}})),
        // --- Asset Librarian ---
        ("al_tag",
         "Asset Librarian: set/clear the tags (flags.asset-librarian.categoryTag / filterTag) on a document. Empty string clears a tag. collection defaults to journal.",
         json!({"type":"object","properties":{
            "_id":{"type":"string"},"name":{"type":"string"},"collection":{"type":"string"},
            "category_tag":{"type":"string"},"filter_tag":{"type":"string"}}})),
        ("al_find",
         "Asset Librarian: find documents whose Asset Librarian tag matches (substring, case-insensitive). field: filter (default) or category. collection defaults to journal.",
         json!({"type":"object","properties":{
            "tag":{"type":"string"},"field":{"type":"string","enum":["filter","category"]},
            "collection":{"type":"string"}},"required":["tag"]})),
        ("client_al_open",
         "Asset Librarian (companion): open the asset browser on the GM client. mode: world/compendium; tab: Actor/Item/JournalEntry/Scene/rolltables… ; optional filters.",
         json!({"type":"object","properties":{
            "mode":{"type":"string"},"tab":{"type":"string"},
            "filters":{"type":"array","items":{"type":"object","additionalProperties":true}}}})),
        // --- Mini Calendar ---
        ("mc_get_time",
         "Mini Calendar / core: read the world time (core.time, in seconds).",
         json!({"type":"object","properties":{}})),
        ("mc_set_time",
         "Mini Calendar / core: set the world time. Give world_time (absolute seconds) or advance_seconds (delta). The calendar updates on the world-time change.",
         json!({"type":"object","properties":{
            "world_time":{"type":"number"},"advance_seconds":{"type":"number"}}})),
        ("mc_list_notes",
         "Mini Calendar: read the calendar note journals (which: player | events | both). Notes are stored as journal pages.",
         json!({"type":"object","properties":{"which":{"type":"string","enum":["player","events","both"]}}})),
        ("client_mc_set_time",
         "Mini Calendar (companion): set the time via the game clock, including \"dawn\"/\"dusk\" of the current/next day (uses the module's setTime macro).",
         json!({"type":"object","properties":{
            "world_time":{"type":"number"},"advance_seconds":{"type":"number"},"mode":{"type":"string","enum":["dawn","dusk"]}}})),
        ("client_mc_open",
         "Mini Calendar (companion): open/toggle the calendar window on the GM client.",
         json!({"type":"object","properties":{}})),
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
                where_.insert(
                    "flags.campaign-codex.data.tags__contains".into(),
                    json!(tag),
                );
            }
            let sheets = foundry
                .get_documents("journal", Some(&where_), None, 0, None)
                .await?;
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
            let ident = str_arg(args, "_id")
                .or_else(|| str_arg(args, "name"))
                .ok_or_else(|| anyhow!("Must provide one of: _id or name"))?;
            let sheet = get_sheet(state, &ident)
                .await?
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
                bail!(
                    "Unknown Campaign Codex type '{cc_type}'. Valid: {}",
                    CC_TYPES.join(", ")
                );
            }
            let description = str_arg(args, "description").unwrap_or_default();
            let mut data = data_defaults(&cc_type);
            data["description"] = json!(description);
            if let (Some(tags), Some(_)) = (args.get("tags"), data.get("tags")) {
                data["tags"] = tags.clone();
            }
            if let Some(actor_arg) = str_arg(args, "linked_actor")
                && data.get("linkedActor").is_some()
            {
                let fields = vec!["_id".into(), "name".into()];
                let actor = foundry
                    .find_document("actors", &actor_arg, Some(&fields))
                    .await?
                    .ok_or_else(|| anyhow!("linked_actor not found: {actor_arg}"))?;
                data["linkedActor"] =
                    json!(format!("Actor.{}", actor["_id"].as_str().unwrap_or("")));
            }
            let mut flags_cc = json!({"type": cc_type, "data": data});
            if let Some(img) = str_arg(args, "image") {
                flags_cc["image"] = json!(img);
            }
            let gm_only = args
                .get("gm_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
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
            let created_id = result
                .pointer("/result/0/_id")
                .cloned()
                .unwrap_or(Value::Null);
            Ok(text_response(&json!({
                "created": {"_id": created_id, "name": sheet_name, "type": cc_type},
                "result": result,
            })))
        }
        "cc_link" => {
            let from_arg = str_arg(args, "from").ok_or_else(|| anyhow!("'from' is required"))?;
            let to_arg = str_arg(args, "to").ok_or_else(|| anyhow!("'to' is required"))?;
            let relation = str_arg(args, "relation").unwrap_or_else(|| "associates".into());
            let from = get_sheet(state, &from_arg)
                .await?
                .ok_or_else(|| anyhow!("Source sheet not found: {from_arg}"))?;
            let from_data = get_path(&from, "flags.campaign-codex.data")
                .cloned()
                .unwrap_or(json!({}));

            let (target_ref, target_label) = if relation == "linkedActor" {
                let fields = vec!["_id".into(), "name".into()];
                let actor = foundry
                    .find_document("actors", &to_arg, Some(&fields))
                    .await?
                    .ok_or_else(|| anyhow!("Actor not found: {to_arg}"))?;
                (
                    format!("Actor.{}", actor["_id"].as_str().unwrap_or("")),
                    actor["name"].as_str().unwrap_or("").to_string(),
                )
            } else {
                let to = get_sheet(state, &to_arg)
                    .await?
                    .ok_or_else(|| anyhow!("Target sheet not found: {to_arg}"))?;
                if args
                    .get("bidirectional")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    let to_assoc = get_path(&to, "flags.campaign-codex.data.associates")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let from_ref = json!(format!(
                        "JournalEntry.{}",
                        from["_id"].as_str().unwrap_or("")
                    ));
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
                (
                    format!("JournalEntry.{}", to["_id"].as_str().unwrap_or("")),
                    to["name"].as_str().unwrap_or("").to_string(),
                )
            };

            let update = if relation == "linkedActor" || relation == "linkedLocation" {
                json!({relation.clone(): target_ref})
            } else {
                let list = from_data
                    .get(&relation)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
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

        // ---------------------------------------------------- Campaign Codex (compagnon)
        "client_cc_convert" => {
            let uuid = str_arg(args, "uuid").ok_or_else(|| anyhow!("'uuid' is required"))?;
            let cc_type = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            let pages = args
                .get("pages_to_separate_sheets")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let r = call_companion(
                state,
                "cc_convert",
                json!({ "uuid": uuid, "type": cc_type, "pagesToSeparateSheets": pages }),
                None,
                30,
            )
            .await?;
            Ok(text_response(&r))
        }
        "client_cc_export_obsidian" => {
            let r = call_companion(state, "cc_export_obsidian", json!({}), None, 60).await?;
            Ok(text_response(&r))
        }
        "client_cc_open_toc" => {
            let mut a = json!({});
            if let Some(t) = str_arg(args, "tab") {
                a["tab"] = json!(t);
            }
            let r = call_companion(state, "cc_open_toc", a, None, 10).await?;
            Ok(text_response(&r))
        }

        // ---------------------------------------------------- Asset Librarian
        "al_tag" => {
            let collection = str_arg(args, "collection").unwrap_or_else(|| "journal".into());
            let ident = str_arg(args, "_id")
                .or_else(|| str_arg(args, "name"))
                .ok_or_else(|| anyhow!("Must provide _id or name"))?;
            let doc = foundry
                .find_document(&collection, &ident, Some(&["_id".into(), "name".into()]))
                .await?
                .ok_or_else(|| anyhow!("Document not found: {ident}"))?;
            let mut flags = serde_json::Map::new();
            if let Some(c) = str_arg(args, "category_tag") {
                flags.insert("categoryTag".into(), json!(c));
            }
            if let Some(f) = str_arg(args, "filter_tag") {
                flags.insert("filterTag".into(), json!(f));
            }
            if flags.is_empty() {
                bail!("Provide category_tag and/or filter_tag");
            }
            let doc_type = crate::foundry::documents::collection_to_type(&collection)
                .ok_or_else(|| anyhow!("Unknown collection: {collection}"))?;
            let result = foundry
                .modify_document(
                    doc_type,
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{"_id": doc["_id"], "flags": {"asset-librarian": flags}}],
                    }),
                )
                .await?;
            Ok(text_response(
                &json!({"document": {"_id": doc["_id"], "name": doc["name"]}, "tags": flags, "result": result}),
            ))
        }
        "al_find" => {
            let tag = str_arg(args, "tag").ok_or_else(|| anyhow!("'tag' is required"))?;
            let collection = str_arg(args, "collection").unwrap_or_else(|| "journal".into());
            let field = if str_arg(args, "field").as_deref() == Some("category") {
                "categoryTag"
            } else {
                "filterTag"
            };
            let mut where_ = serde_json::Map::new();
            where_.insert(
                format!("flags.asset-librarian.{field}__contains"),
                json!(tag),
            );
            let docs = foundry
                .get_documents(
                    &collection,
                    Some(&where_),
                    Some(&["_id".into(), "name".into()]),
                    0,
                    None,
                )
                .await?;
            Ok(text_response(
                &json!({"field": field, "tag": tag, "count": docs.len(), "matches": docs}),
            ))
        }
        "client_al_open" => {
            let mut a = json!({
                "mode": str_arg(args, "mode").unwrap_or_else(|| "world".into()),
                "tab": str_arg(args, "tab").unwrap_or_else(|| "Item".into()),
            });
            if let Some(f) = args.get("filters") {
                a["filters"] = f.clone();
            }
            let r = call_companion(state, "al_open", a, None, 10).await?;
            Ok(text_response(&r))
        }

        // ---------------------------------------------------- Mini Calendar
        "mc_get_time" => {
            let w = json!({"key": "core.time"}).as_object().cloned().unwrap();
            let docs = foundry
                .get_documents("settings", Some(&w), None, 0, Some(1))
                .await?;
            let raw = docs
                .first()
                .and_then(|d| d.get("value"))
                .and_then(Value::as_str)
                .unwrap_or("0");
            let seconds: i64 = raw.trim().parse().unwrap_or(0);
            Ok(text_response(&json!({"worldTime": seconds})))
        }
        "mc_set_time" => {
            let w = json!({"key": "core.time"}).as_object().cloned().unwrap();
            let docs = foundry
                .get_documents("settings", Some(&w), None, 0, Some(1))
                .await?;
            let doc = docs
                .first()
                .ok_or_else(|| anyhow!("core.time setting not found"))?;
            let current: i64 = doc
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("0")
                .trim()
                .parse()
                .unwrap_or(0);
            let target = match (
                args.get("world_time").and_then(Value::as_i64),
                args.get("advance_seconds").and_then(Value::as_i64),
            ) {
                (Some(t), _) => t,
                (None, Some(d)) => current + d,
                _ => bail!("Provide world_time or advance_seconds"),
            };
            let result = foundry
                .modify_document(
                    "Setting",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{"_id": doc["_id"], "value": target.to_string()}],
                    }),
                )
                .await?;
            Ok(text_response(
                &json!({"worldTime": target, "before": current, "result": result}),
            ))
        }
        "mc_list_notes" => {
            let which = str_arg(args, "which").unwrap_or_else(|| "both".into());
            let mut out = serde_json::Map::new();
            for (key, jname) in MC_NOTE_JOURNALS {
                if which != "both" && which != key {
                    continue;
                }
                let doc = foundry
                    .get_document("journal", None, Some(jname), None)
                    .await?;
                let pages = doc
                    .as_ref()
                    .and_then(|d| d.get("pages"))
                    .and_then(Value::as_array)
                    .map(|ps| {
                        ps.iter()
                            .map(|p| {
                                json!({
                                    "name": p.get("name"),
                                    "text": p.pointer("/text/content"),
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                out.insert(
                    key.to_string(),
                    json!({"journal": jname, "found": doc.is_some(), "pages": pages}),
                );
            }
            Ok(text_response(&Value::Object(out)))
        }
        "client_mc_set_time" => {
            let mut a = json!({});
            for k in ["world_time", "advance_seconds"] {
                if let Some(v) = args.get(k) {
                    a[k] = v.clone();
                }
            }
            if let Some(m) = str_arg(args, "mode") {
                a["mode"] = json!(m);
            }
            let r = call_companion(state, "mc_set_time", a, None, 10).await?;
            Ok(text_response(&r))
        }
        "client_mc_open" => {
            let r = call_companion(state, "mc_open", json!({}), None, 10).await?;
            Ok(text_response(&r))
        }

        other => bail!("Unknown tool: {other}"),
    }
}
