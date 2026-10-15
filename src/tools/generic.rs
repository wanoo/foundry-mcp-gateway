//! Les outils GÉNÉRIQUES : ceux qui ne dépendent d'aucun système de jeu ni
//! d'aucun addon — lectures/écritures de collections, packs, fichiers,
//! compendiums, recherche, événements.
//!
//! Séparés du registre pour que `tools/mod.rs` reste ce qu'il doit être : le
//! point d'entrée qu'un contributeur ouvre en premier (définitions, dispatch,
//! helpers de réponse), et non un fourre-tout de 1200 lignes.

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use super::{
    COLLECTIONS, fields_arg, plural_to_collection, str_arg, text_response, truncate_by_bytes,
    where_arg,
};
use crate::foundry::documents::{can_use_index, filter_fields};
use crate::mcp::McpState;

pub(super) async fn run_tool(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;

    // Paires générées get_<pluriel> / get_<singulier>
    for (plural, singular, _) in COLLECTIONS {
        if name == format!("get_{plural}") {
            let fields = fields_arg(args);
            let where_ = where_arg(args);
            let docs = foundry
                .get_documents(
                    plural_to_collection(plural),
                    where_.as_ref(),
                    fields.as_deref(),
                    args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
                    args.get("limit")
                        .and_then(Value::as_u64)
                        .map(|l| l as usize),
                )
                .await?;
            let docs = truncate_by_bytes(
                docs,
                args.get("max_length")
                    .and_then(Value::as_u64)
                    .map(|m| m as usize),
            );
            return Ok(text_response(&Value::Array(docs)));
        }
        if name == format!("get_{singular}") {
            let id = str_arg(args, "_id").or_else(|| str_arg(args, "id"));
            let doc_name = str_arg(args, "name");
            if id.is_none() && doc_name.is_none() {
                bail!("Must provide one of: _id, id, or name");
            }
            let fields = fields_arg(args);
            let doc = foundry
                .get_document(
                    plural_to_collection(plural),
                    id.as_deref(),
                    doc_name.as_deref(),
                    fields.as_deref(),
                )
                .await?;
            return Ok(match doc {
                Some(d) => text_response(&d),
                None => text_response(&json!(format!("{singular} not found"))),
            });
        }
    }

    match name {
        "ping" => {
            let hostname = foundry.hostname().to_string();
            let status = crate::foundry::auth_status(&foundry.http, &hostname).await;
            Ok(text_response(&json!({
                "connected": foundry.is_connected(),
                "hostname": hostname,
                "userId": foundry.user_id().await,
                "generation": foundry.generation().await,
                "eventSeq": foundry.event_seq(),
                "server": status,
            })))
        }
        "get_world" => {
            let mut world = foundry.request_world().await?;
            if let Some(obj) = world.as_object_mut() {
                for key in [
                    "actors",
                    "items",
                    "folders",
                    "users",
                    "scenes",
                    "journal",
                    "macros",
                    "cards",
                    "playlists",
                    "tables",
                    "combats",
                    "messages",
                    "packs",
                ] {
                    obj.remove(key);
                }
            }
            Ok(text_response(&world))
        }
        "get_current_scene" => {
            let where_: Map<String, Value> = json!({"active": true}).as_object().cloned().unwrap();
            let fields = fields_arg(args).unwrap_or_else(|| {
                [
                    "_id",
                    "name",
                    "active",
                    "navigation",
                    "navName",
                    "background",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect()
            });
            let docs = foundry
                .get_documents("scenes", Some(&where_), Some(&fields), 0, Some(1))
                .await?;
            Ok(match docs.into_iter().next() {
                Some(d) => text_response(&d),
                None => {
                    text_response(&json!({"active": null, "note": "No scene is currently active"}))
                }
            })
        }
        "search_journals" => {
            let query = str_arg(args, "query").ok_or_else(|| anyhow!("'query' is required"))?;
            let max = args
                .get("max_results")
                .and_then(Value::as_u64)
                .unwrap_or(20) as usize;
            let docs = foundry
                .get_documents("journal", None, None, 0, None)
                .await?;
            let needle = query.to_lowercase();
            let strip = regex::Regex::new(r"<[^>]+>").unwrap();
            let mut hits = Vec::new();
            'outer: for j in &docs {
                let jname = j.get("name").and_then(Value::as_str).unwrap_or("");
                if jname.to_lowercase().contains(&needle) {
                    hits.push(json!({"_id": j["_id"], "name": jname, "match": "name"}));
                }
                for p in j.get("pages").and_then(Value::as_array).unwrap_or(&vec![]) {
                    if hits.len() >= max {
                        break 'outer;
                    }
                    let content = p
                        .pointer("/text/content")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let text = strip.replace_all(content, " ");
                    let lower = text.to_lowercase();
                    if let Some(idx) = lower.find(&needle) {
                        let start = idx.saturating_sub(80);
                        let end = (idx + needle.len() + 80).min(text.len());
                        let snippet: String = text
                            .char_indices()
                            .filter(|(i, _)| *i >= start && *i < end)
                            .map(|(_, c)| c)
                            .collect();
                        hits.push(json!({
                            "_id": j["_id"], "name": jname, "match": "content",
                            "page": {"_id": p["_id"], "name": p["name"]},
                            "snippet": snippet.split_whitespace().collect::<Vec<_>>().join(" "),
                        }));
                    }
                }
                if hits.len() >= max {
                    break;
                }
            }
            Ok(text_response(&Value::Array(hits)))
        }
        "list_compendium_packs" => {
            let world = foundry.request_world().await?;
            let packs = world
                .get("packs")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .map(|p| {
                            json!({
                                "id": p.get("id").or(p.get("collection")).cloned(),
                                "label": p.get("label").cloned(),
                                "type": p.get("type").or(p.get("documentName")).cloned(),
                                "system": p.get("system").cloned(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(text_response(&Value::Array(packs)))
        }
        "get_pack_documents" => {
            let doc_type = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            let pack = str_arg(args, "pack").ok_or_else(|| anyhow!("'pack' is required"))?;
            let query = args
                .get("query")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let fields = fields_arg(args);
            let use_index = can_use_index(fields.as_deref(), Some(&query));
            let mut docs = foundry
                .get_collection(&doc_type, query.clone(), use_index, Some(&pack))
                .await?;
            if use_index && !docs.is_empty() && docs.iter().any(|d| d.get("_id").is_none()) {
                docs = foundry
                    .get_collection(&doc_type, query, false, Some(&pack))
                    .await?;
            }
            let out: Vec<Value> = docs
                .iter()
                .map(|d| filter_fields(d, fields.as_deref()))
                .collect();
            let out = truncate_by_bytes(
                out,
                args.get("max_length")
                    .and_then(Value::as_u64)
                    .map(|m| m as usize),
            );
            Ok(text_response(&Value::Array(out)))
        }
        "create_document" | "modify_document" | "delete_document" => {
            let doc_type = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            let mut op = json!({ "broadcast": false });
            if let Some(p) = str_arg(args, "parent_uuid") {
                op["parentUuid"] = json!(p);
            }
            if let Some(p) = str_arg(args, "pack") {
                op["pack"] = json!(p);
            }
            let action = match name {
                "create_document" => {
                    let data = args
                        .get("data")
                        .and_then(Value::as_array)
                        .ok_or_else(|| anyhow!("'data' (array) is required"))?;
                    op["data"] = json!(data);
                    op["keepId"] = json!(
                        args.get("keep_id")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    );
                    op["renderSheet"] = json!(false);
                    "create"
                }
                "modify_document" => {
                    let id = str_arg(args, "_id").ok_or_else(|| anyhow!("'_id' is required"))?;
                    let updates = args
                        .get("updates")
                        .and_then(Value::as_array)
                        .ok_or_else(|| anyhow!("'updates' (array) is required"))?;
                    let with_ids: Vec<Value> = updates
                        .iter()
                        .map(|u| {
                            let mut u = u.clone();
                            u["_id"] = json!(id);
                            u
                        })
                        .collect();
                    op["updates"] = json!(with_ids);
                    op["diff"] = json!(false);
                    op["recursive"] = json!(true);
                    "update"
                }
                _ => {
                    let ids = args
                        .get("ids")
                        .and_then(Value::as_array)
                        .ok_or_else(|| anyhow!("'ids' (array) is required"))?;
                    op["ids"] = json!(ids);
                    "delete"
                }
            };
            op["action"] = json!(action);
            let result = foundry.modify_document(&doc_type, action, op).await?;
            Ok(text_response(&result))
        }
        "get_events" => {
            let since = args.get("since_seq").and_then(Value::as_u64).unwrap_or(0);
            let event = str_arg(args, "event");
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map(|l| l as usize);
            let (last_seq, events) = foundry.get_events(since, event.as_deref(), limit).await;
            let events: Vec<Value> = events
                .into_iter()
                .map(|e| json!({"seq": e.seq, "event": e.event, "args": e.args}))
                .collect();
            Ok(text_response(
                &json!({"lastSeq": last_seq, "count": events.len(), "events": events}),
            ))
        }
        _ => bail!("Unknown tool: {name}"),
    }
}
