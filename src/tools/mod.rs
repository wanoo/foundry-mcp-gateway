//! Registre d'outils MCP — noyau générique. (Portage progressif : ce module
//! couvre lectures/écritures/packs/recherche/événements ; les outils de séance
//! et les modules système arrivent par lots.)

pub mod addons;
pub mod admin;
pub mod cc_family;
pub mod companion;
pub mod insight;
pub mod manage;
pub mod markdown;
pub mod session;
pub mod table;
pub mod transfer;

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::foundry::documents::{can_use_index, filter_fields, get_path};
use crate::mcp::McpState;
use crate::systems;

pub const COLLECTIONS: [(&str, &str, &str); 13] = [
    ("actors", "actor", "actor"),
    ("items", "item", "item"),
    ("folders", "folder", "folder"),
    ("users", "user", "user"),
    ("scenes", "scene", "scene"),
    ("journals", "journal", "journal entry"),
    ("macros", "macro", "macro"),
    ("cards", "card", "cards stack"),
    ("playlists", "playlist", "playlist"),
    ("tables", "table", "roll table"),
    ("combats", "combat", "combat"),
    ("messages", "message", "chat message"),
    ("settings", "setting", "setting"),
];

/// pluriel d'outil → nom de collection Foundry.
pub fn plural_to_collection(plural: &str) -> &str {
    if plural == "journals" {
        "journal"
    } else {
        plural
    }
}

pub fn text_response(value: &Value) -> Value {
    let mut out = json!({ "content": [{ "type": "text", "text": value.to_string() }] });
    // MCP 2025-06-18 : les clients récents lisent structuredContent (objets
    // seulement, par spec) ; le texte reste pour les clients plus anciens.
    if value.is_object() {
        out["structuredContent"] = value.clone();
    }
    out
}

/// Contenu image MCP natif (base64 brut, sans préfixe data:) + une légende texte.
pub fn image_response(base64: &str, mime_type: &str, caption: &Value) -> Value {
    json!({ "content": [
        { "type": "image", "data": base64, "mimeType": mime_type },
        { "type": "text", "text": caption.to_string() },
    ]})
}

pub fn error_response(message: String) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

/// Poste un ChatMessage (author = le bot, whisper optionnel) — helper partagé.
pub async fn post_chat(
    state: &McpState,
    content: &str,
    flags: Value,
    whisper: Option<&Value>,
) -> Result<Value> {
    let mut message = json!({
        "content": content,
        "author": state.foundry.user_id().await,
        "flags": flags,
    });
    if let Some(w) = whisper.and_then(Value::as_array)
        && !w.is_empty()
    {
        message["whisper"] = json!(w);
    }
    state
        .foundry
        .modify_document(
            "ChatMessage",
            "create",
            json!({
                "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                "data": [message],
            }),
        )
        .await
}

/// Tirages sur une RollTable (formule NdM±k + modificateur, sélection par plage).
pub fn roll_table_draws(table: &Value, modifier: i64, rolls: usize) -> Result<Vec<Value>> {
    let formula = table
        .get("formula")
        .and_then(Value::as_str)
        .unwrap_or("1d100")
        .replace(char::is_whitespace, "");
    let re = regex::Regex::new(r"^(?i)(\d+)d(\d+)([+-]\d+)?$").unwrap();
    let caps = re
        .captures(&formula)
        .ok_or_else(|| anyhow!("Unsupported table formula '{formula}' (expected NdM±k)"))?;
    let n: i64 = caps[1].parse()?;
    let m: i64 = caps[2].parse()?;
    let k: i64 = caps
        .get(3)
        .map(|c| c.as_str().parse().unwrap_or(0))
        .unwrap_or(0);
    let empty = vec![];
    let results = table
        .get("results")
        .and_then(Value::as_array)
        .unwrap_or(&empty);

    let mut draws = Vec::new();
    for _ in 0..rolls {
        let mut roll = k + modifier;
        for _ in 0..n {
            roll += 1 + (rand::random::<f64>() * m as f64) as i64 % m;
        }
        let hit = results.iter().find(|r| {
            r.get("range")
                .and_then(Value::as_array)
                .and_then(|rg| Some((rg.first()?.as_i64()?, rg.get(1)?.as_i64()?)))
                .map(|(lo, hi)| roll >= lo && roll <= hi)
                .unwrap_or(false)
        });
        draws.push(json!({
            "roll": roll,
            "range": hit.and_then(|h| h.get("range")).cloned(),
            "text": hit.and_then(|h| h.get("description").or(h.get("text")).or(h.get("name")))
                .and_then(Value::as_str),
            "documentUuid": hit.and_then(|h| h.get("documentUuid")).cloned(),
        }));
    }
    Ok(draws)
}

/// Troncature par taille JSON (compat `max_length` du serveur TS).
fn truncate_by_bytes(mut docs: Vec<Value>, max_length: Option<usize>) -> Vec<Value> {
    let Some(max) = max_length.filter(|m| *m > 0) else {
        return docs;
    };
    while !docs.is_empty() && Value::Array(docs.clone()).to_string().len() > max {
        docs.pop();
    }
    docs
}

/// Un outil sans effet de bord sur le monde (sert aux annotations ET au mode
/// FOUNDRY_READONLY). ⚠️ manage_modules/manage_users ÉCRIVENT : jamais ici.
pub fn is_read_only(name: &str) -> bool {
    name.starts_with("get_")
        || name.starts_with("list_")
        || matches!(
            name,
            "admin_status"
                | "admin_check_package"
                | "admin_list_backups"
                | "search_journals"
                | "ping"
                | "export_journals"
                | "client_status"
                | "client_get_state"
                | "client_get_derived"
                | "client_enrich"
                | "client_search"
                | "client_capture"
                | "client_scene_report"
                | "client_babele"
                | "client_weather_types"
                | "client_token_fx_presets"
                | "client_effect_catalog"
        )
}

fn annotations(name: &str) -> Value {
    let destructive = matches!(name, "delete_document" | "delete_compendium");
    json!({ "readOnlyHint": is_read_only(name), "destructiveHint": destructive })
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    // Chaque outil accepte `instance` : le monde visé (voir show_credentials).
    let mut schema = schema;
    if !matches!(
        name,
        "copy_documents" | "show_credentials" | "choose_foundry_instance"
    ) && let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut)
    {
        props.insert(
            "instance".to_string(),
            json!({"type":"string","description":"target Foundry instance _id (default: the active one) — several worlds can be served at once"}),
        );
    }
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
        "annotations": annotations(name),
    })
}

pub fn definitions(state: &McpState) -> Vec<Value> {
    let mut tools = Vec::new();
    // (mode lecture seule : filtrage en fin de fonction)
    for (plural, singular, label) in COLLECTIONS {
        tools.push(tool(
            &format!("get_{plural}"),
            &format!("Get all {label}s. Filter with `where` (dotted paths + __in/__contains/__ne/__exists), project with requested_fields, paginate with offset/limit."),
            json!({ "type": "object", "properties": {
                "where": { "type": "object", "additionalProperties": true },
                "requested_fields": { "type": "array", "items": { "type": "string" } },
                "offset": { "type": "number" },
                "limit": { "type": "number" },
                "max_length": { "type": "number", "description": "Truncate the response to ~this many JSON bytes (drops trailing docs)" },
            }}),
        ));
        tools.push(tool(
            &format!("get_{singular}"),
            &format!("Get one {label} by _id or name."),
            json!({ "type": "object", "properties": {
                "_id": { "type": "string" },
                "id": { "type": "string" },
                "name": { "type": "string" },
                "requested_fields": { "type": "array", "items": { "type": "string" } },
            }}),
        ));
    }
    tools.push(tool(
        "ping",
        "Lightweight health check: Foundry /api/status + connection state (no world dump).",
        json!({ "type": "object", "properties": {} }),
    ));
    tools.push(tool(
        "get_world",
        "World metadata (title, system, modules) — the only heavy full-dump call; prefer ping for liveness.",
        json!({ "type": "object", "properties": {} }),
    ));
    tools.push(tool(
        "get_current_scene",
        "The currently active scene.",
        json!({ "type": "object", "properties": {
            "requested_fields": { "type": "array", "items": { "type": "string" } },
        }}),
    ));
    tools.push(tool(
        "search_journals",
        "Full-text search across journal names and page contents (HTML stripped, case-insensitive).",
        json!({ "type": "object", "properties": {
            "query": { "type": "string" },
            "max_results": { "type": "number" },
        }, "required": ["query"] }),
    ));
    tools.push(tool(
        "list_compendium_packs",
        "List the world's compendium packs (id, label, type).",
        json!({ "type": "object", "properties": {} }),
    ));
    tools.push(tool(
        "get_pack_documents",
        "Read documents from a compendium pack (index auto for _id/name listings).",
        json!({ "type": "object", "properties": {
            "type": { "type": "string" },
            "pack": { "type": "string" },
            "query": { "type": "object", "additionalProperties": true },
            "requested_fields": { "type": "array", "items": { "type": "string" } },
            "max_length": { "type": "number" },
        }, "required": ["type", "pack"] }),
    ));
    tools.push(tool(
        "create_document",
        "Create documents (data = ARRAY even for one). parent_uuid for embedded, pack for compendia, keep_id to preserve _ids.",
        json!({ "type": "object", "properties": {
            "type": { "type": "string" },
            "data": { "type": "array", "items": { "type": "object" } },
            "parent_uuid": { "type": "string" },
            "pack": { "type": "string" },
            "keep_id": { "type": "boolean" },
        }, "required": ["type", "data"] }),
    ));
    tools.push(tool(
        "modify_document",
        "Update a document (dotted-key updates merge into the document).",
        json!({ "type": "object", "properties": {
            "type": { "type": "string" },
            "_id": { "type": "string" },
            "updates": { "type": "array", "items": { "type": "object" } },
            "parent_uuid": { "type": "string" },
            "pack": { "type": "string" },
        }, "required": ["type", "_id", "updates"] }),
    ));
    tools.push(tool(
        "delete_document",
        "Delete documents by ids (ARRAY). Permanent.",
        json!({ "type": "object", "properties": {
            "type": { "type": "string" },
            "ids": { "type": "array", "items": { "type": "string" } },
            "parent_uuid": { "type": "string" },
            "pack": { "type": "string" },
        }, "required": ["type", "ids"] }),
    ));
    tools.push(tool(
        "get_events",
        "Buffered socket broadcasts from OTHER clients (chat, writes, combat). Poll incrementally with since_seq.",
        json!({ "type": "object", "properties": {
            "since_seq": { "type": "number" },
            "event": { "type": "string" },
            "limit": { "type": "number" },
        }}),
    ));
    // Outils de séance, gestion, Campaign Codex, et modules système.
    for (name, desc, schema) in session::definitions()
        .into_iter()
        .chain(manage::definitions())
        .chain(cc_family::definitions())
        .chain(addons::definitions())
        .chain(insight::definitions())
        .chain(table::definitions())
        .chain(admin::definitions(state))
        .chain(transfer::definitions())
        .chain(companion::definitions())
        .chain(
            systems::loaded_modules()
                .iter()
                .flat_map(|m| (m.definitions)()),
        )
    {
        tools.push(tool(name, desc, schema));
    }
    if state.readonly {
        tools.retain(|t| {
            t.get("name")
                .and_then(Value::as_str)
                .is_some_and(is_read_only)
        });
    }
    tools
}

pub fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(String::from)
}

pub fn fields_arg(args: &Value) -> Option<Vec<String>> {
    args.get("requested_fields")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
}

pub fn where_arg(args: &Value) -> Option<Map<String, Value>> {
    args.get("where").and_then(Value::as_object).cloned()
}

pub async fn dispatch(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    // Multi-mondes : `instance` choisit la connexion Foundry visée. Comme tous
    // les outils passent par `state.foundry`, il suffit de la substituer ici.
    let routed;
    let state = match args.get("instance").and_then(Value::as_str) {
        Some(inst) => match state.resolve(Some(inst)).await {
            Ok(s) => {
                routed = s;
                &routed
            }
            Err(e) => return Ok(error_response(format!("Error: {e:#}"))),
        },
        None => state,
    };
    if state.readonly && !is_read_only(name) {
        return Ok(error_response(format!(
            "Error: '{name}' modifies the world and this gateway runs in read-only mode \
             (FOUNDRY_READONLY). Only read-only tools are available."
        )));
    }
    let result = if session::handles(name) {
        session::run(state, name, args).await
    } else if manage::handles(name) {
        manage::run(state, name, args).await
    } else if cc_family::handles(name) {
        cc_family::run(state, name, args).await
    } else if addons::handles(name) {
        addons::run(state, name, args).await
    } else if transfer::handles(name) {
        transfer::run(state, name, args).await
    } else if admin::handles(name) {
        admin::run(state, name, args).await
    } else if table::handles(name) {
        table::run(state, name, args).await
    } else if insight::handles(name) {
        insight::run(state, name, args).await
    } else if companion::handles(name) {
        companion::run(state, name, args).await
    } else if systems::loaded_modules().iter().any(|m| (m.handles)(name)) {
        systems::run(state, name, args).await
    } else {
        run_tool(state, name, args).await
    };
    match result {
        Ok(v) => Ok(v),
        Err(e) => Ok(error_response(format!("Error: {e:#}"))),
    }
}

async fn run_tool(state: &McpState, name: &str, args: &Value) -> Result<Value> {
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

// --- Ressources MCP ------------------------------------------------------------

const PAGE_SIZE: usize = 100;

pub async fn resources_list(state: &McpState, cursor: Option<&str>) -> Result<Value> {
    let (section, offset) = match cursor {
        None => ("a", 0),
        Some(c) => {
            let (s, o) = c
                .split_once(':')
                .ok_or_else(|| anyhow!("Invalid cursor: {c}"))?;
            (
                match s {
                    "a" => "a",
                    "j" => "j",
                    _ => bail!("Invalid cursor: {c}"),
                },
                o.parse::<usize>().unwrap_or(0),
            )
        }
    };
    let fields = vec!["_id".to_string(), "name".to_string()];
    let (collection, uri_section, mime, next_section) = match section {
        "a" => ("actors", "actors", "application/json", Some("j:0")),
        _ => ("journal", "journal", "text/html", None),
    };
    let docs = state
        .foundry
        .get_documents(collection, None, Some(&fields), 0, None)
        .await?;
    let page: Vec<Value> = docs
        .iter()
        .skip(offset)
        .take(PAGE_SIZE)
        .map(|d| {
            json!({
                "uri": format!("foundry://{uri_section}/{}", d["_id"].as_str().unwrap_or("")),
                "name": d["name"],
                "mimeType": mime,
            })
        })
        .collect();
    let next = if offset + PAGE_SIZE < docs.len() {
        Some(format!("{section}:{}", offset + PAGE_SIZE))
    } else {
        next_section.map(String::from)
    };
    let mut out = json!({ "resources": page });
    if let Some(n) = next {
        out["nextCursor"] = json!(n);
    }
    Ok(out)
}

pub async fn resources_read(state: &McpState, uri: &str) -> Result<Value> {
    let re = regex::Regex::new(r"^foundry://(actors|journal)/([A-Za-z0-9]+)$").unwrap();
    let caps = re
        .captures(uri)
        .ok_or_else(|| anyhow!("Unknown resource URI: {uri}"))?;
    let collection = caps.get(1).unwrap().as_str();
    let id = caps.get(2).unwrap().as_str();
    let doc = state
        .foundry
        .get_document(collection, Some(id), None, None)
        .await?
        .ok_or_else(|| anyhow!("Resource not found: {uri}"))?;

    if collection == "actors" {
        return Ok(json!({ "contents": [{
            "uri": uri, "mimeType": "application/json", "text": doc.to_string(),
        }]}));
    }
    let name = doc.get("name").and_then(Value::as_str).unwrap_or("");
    let mut html = format!("<h1>{name}</h1>");
    for p in doc
        .get("pages")
        .and_then(Value::as_array)
        .unwrap_or(&vec![])
    {
        let pname = p.get("name").and_then(Value::as_str).unwrap_or("");
        let content = p
            .pointer("/text/content")
            .and_then(Value::as_str)
            .unwrap_or("");
        html.push_str(&format!("\n<h2>{pname}</h2>\n{content}"));
    }
    let mut contents = vec![json!({ "uri": uri, "mimeType": "text/html", "text": html })];
    if let Some(cc) = get_path(&doc, "flags.campaign-codex") {
        contents.push(json!({
            "uri": format!("{uri}#campaign-codex"),
            "mimeType": "application/json",
            "text": cc.to_string(),
        }));
    }
    Ok(json!({ "contents": contents }))
}

// --- Prompts MCP ---------------------------------------------------------------

pub fn prompt_definitions() -> Vec<Value> {
    vec![
        json!({ "name": "session-recap",
            "description": "Résumer la dernière séance à partir des messages de chat récents.",
            "arguments": [{ "name": "max_messages", "required": false }] }),
        json!({ "name": "world-overview",
            "description": "Brief de l'état du monde : scène active, combats, joueurs connectés.",
            "arguments": [] }),
        json!({ "name": "prep-checklist",
            "description": "Vérifier la préparation d'une scène : tokens, playlists — et lister ce qui manque.",
            "arguments": [{ "name": "scene", "required": false }] }),
    ]
}

pub async fn prompts_get(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    let strip = regex::Regex::new(r"<[^>]+>").unwrap();
    match name {
        "session-recap" => {
            let max = args
                .get("max_messages")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(50);
            let messages = foundry
                .get_documents("messages", None, None, 0, None)
                .await?;
            let recent: Vec<String> = messages
                .iter()
                .rev()
                .take(max)
                .rev()
                .map(|m| {
                    let content = m.get("content").and_then(Value::as_str).unwrap_or("");
                    format!(
                        "- {}",
                        strip
                            .replace_all(content, " ")
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                })
                .collect();
            Ok(json!({
                "description": format!("Résumé de séance sur {} messages", recent.len()),
                "messages": [{ "role": "user", "content": { "type": "text", "text": format!(
                    "Voici les {} derniers messages du chat (du plus ancien au plus récent) :\n\n{}\n\nRédige un résumé de séance structuré : événements marquants, jets décisifs, décisions, fils ouverts.",
                    recent.len(), recent.join("\n")
                )}}]
            }))
        }
        "world-overview" => {
            let hostname = foundry.hostname().to_string();
            let status = crate::foundry::auth_status(&foundry.http, &hostname).await;
            let fields = vec!["_id".to_string(), "name".to_string()];
            let where_ = json!({"active": true}).as_object().cloned().unwrap();
            let scenes = foundry
                .get_documents("scenes", Some(&where_), Some(&fields), 0, Some(1))
                .await?;
            let combats = foundry
                .get_documents("combats", None, Some(&fields), 0, None)
                .await?;
            let scene = scenes
                .first()
                .and_then(|s| s.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("aucune");
            Ok(json!({
                "description": "Brief de l'état du monde",
                "messages": [{ "role": "user", "content": { "type": "text", "text": format!(
                    "État du monde Foundry « {} » (v{}) :\n- Scène active : {}\n- Combats en cours : {}\n- Joueurs connectés : {}\n\nFais-moi un brief de reprise MJ.",
                    status.as_ref().and_then(|s| s.get("world")).and_then(Value::as_str).unwrap_or("?"),
                    status.as_ref().and_then(|s| s.get("version")).and_then(Value::as_str).unwrap_or("?"),
                    scene,
                    combats.len(),
                    status.as_ref().and_then(|s| s.get("users")).and_then(Value::as_u64).unwrap_or(0),
                )}}]
            }))
        }
        "prep-checklist" => {
            let scene = match args.get("scene").and_then(Value::as_str) {
                Some(s) => {
                    state
                        .foundry
                        .find_document(
                            "scenes",
                            s,
                            Some(&["_id".into(), "name".into(), "tokens".into()]),
                        )
                        .await?
                }
                None => {
                    let w = json!({"active": true}).as_object().cloned().unwrap();
                    state
                        .foundry
                        .get_documents(
                            "scenes",
                            Some(&w),
                            Some(&["_id".into(), "name".into(), "tokens".into()]),
                            0,
                            Some(1),
                        )
                        .await?
                        .into_iter()
                        .next()
                }
            };
            let scene_name = scene
                .as_ref()
                .and_then(|s| s.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let empty = vec![];
            let tokens: Vec<&str> = scene
                .as_ref()
                .and_then(|s| s.get("tokens"))
                .and_then(Value::as_array)
                .unwrap_or(&empty)
                .iter()
                .filter_map(|t| t.get("name").and_then(Value::as_str))
                .collect();
            let fields = vec!["_id".into(), "name".into()];
            let playlists = state
                .foundry
                .get_documents("playlists", None, Some(&fields), 0, None)
                .await?;
            let pl_names: Vec<&str> = playlists
                .iter()
                .filter_map(|p| p.get("name").and_then(Value::as_str))
                .collect();
            Ok(json!({
                "description": format!("Checklist de préparation — {scene_name}"),
                "messages": [{ "role": "user", "content": { "type": "text", "text": format!(
                    "Préparation de la scène « {scene_name} » :\n- Tokens posés ({}) : {}\n- Playlists disponibles : {}\n\nAnalyse cette préparation : qu'est-ce qui manque probablement (adversaires, ambiance, handouts, éclairage) pour jouer cette scène confortablement ?",
                    tokens.len(),
                    if tokens.is_empty() { "aucun".to_string() } else { tokens.join(", ") },
                    if pl_names.is_empty() { "aucune".to_string() } else { pl_names.join(", ") },
                )}}]
            }))
        }
        _ => bail!("Unknown prompt: {name}"),
    }
}
