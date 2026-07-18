//! Gestion : settings, ownership, compendiums, import/export, fichiers,
//! credentials/instances, attente de messages — port des comportements TS.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::markdown::html_to_markdown;
use super::{str_arg, text_response};
use crate::mcp::McpState;

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("set_setting",
         "Set a world-scoped setting (upsert). Keys are namespaced like \"module.setting\".",
         json!({"type":"object","properties":{
            "key":{"type":"string"},"value":{}},"required":["key","value"]})),
        ("list_actor_ownership",
         "Actor permissions: which users own/observe which actors (user names resolved). Levels: none/limited/observer/owner.",
         json!({"type":"object","properties":{"_id":{"type":"string"},"name":{"type":"string"}}})),
        ("set_actor_ownership",
         "Grant or revoke a user's permission on an actor (level \"none\" revokes). default_level sets the actor-wide default.",
         json!({"type":"object","properties":{
            "_id":{"type":"string"},"name":{"type":"string"},"user":{"type":"string"},
            "level":{"type":"string","enum":["none","limited","observer","owner"]},
            "default_level":{"type":"string","enum":["none","limited","observer","owner"]}}})),
        ("import_from_compendium",
         "Import a document from a pack into the world (by _id or name; keep_id, folder).",
         json!({"type":"object","properties":{
            "pack":{"type":"string"},"type":{"type":"string"},
            "_id":{"type":"string"},"name":{"type":"string"},
            "keep_id":{"type":"boolean"},"folder":{"type":"string"}},
            "required":["pack","type"]})),
        ("export_journals",
         "Export journals as Markdown (where filter, offset/limit — default limit 20).",
         json!({"type":"object","properties":{
            "where":{"type":"object","additionalProperties":true},
            "offset":{"type":"number"},"limit":{"type":"number"}}})),
        ("create_compendium",
         "Create a Compendium pack (label + document type).",
         json!({"type":"object","properties":{
            "label":{"type":"string"},"type":{"type":"string"}},"required":["label","type"]})),
        ("delete_compendium",
         "Delete a Compendium pack and ALL its documents. Permanent.",
         json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]})),
        ("create_directory",
         "Create a directory in Foundry's storage (parents must exist).",
         json!({"type":"object","properties":{
            "target":{"type":"string"},"source":{"type":"string"}},"required":["target"]})),
        ("browse_files",
         "Browse Foundry's file storage (dirs + files at a path).",
         json!({"type":"object","properties":{
            "target":{"type":"string"},"type":{"type":"string"},
            "extensions":{"type":"array","items":{"type":"string"}}},"required":["target"]})),
        ("upload_file",
         "Upload a file to Foundry storage. EXACTLY ONE of url or image_data (base64). Target dir must exist (create_directory).",
         json!({"type":"object","properties":{
            "target":{"type":"string"},"filename":{"type":"string"},
            "url":{"type":"string"},"image_data":{"type":"string"}},
            "required":["target","filename"]})),
        ("show_credentials",
         "Configured Foundry credentials (no passwords) + which is active.",
         json!({"type":"object","properties":{}})),
        ("choose_foundry_instance",
         "Switch to another configured Foundry instance (by item_order or _id) — forces reconnection.",
         json!({"type":"object","properties":{
            "item_order":{"type":"number"},"_id":{"type":"string"}}})),
        ("wait_for_message",
         "Block until a ChatMessage created by ANOTHER client arrives (where filter, timeout ≤120 s). Flow: get_events lastSeq → trigger → wait.",
         json!({"type":"object","properties":{
            "where":{"type":"object","additionalProperties":true},
            "timeout_seconds":{"type":"number"},"since_seq":{"type":"number"}}})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

const LEVELS: [(&str, u64); 4] = [("none", 0), ("limited", 1), ("observer", 2), ("owner", 3)];
fn level_num(name: &str) -> Option<u64> {
    LEVELS.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}
fn level_name(v: u64) -> String {
    LEVELS
        .iter()
        .find(|(_, n)| *n == v)
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| v.to_string())
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "set_setting" => {
            let key = str_arg(args, "key").ok_or_else(|| anyhow!("'key' is required"))?;
            let value = args
                .get("value")
                .cloned()
                .ok_or_else(|| anyhow!("'value' is required"))?;
            let serialized = value.to_string();
            let w = json!({"key": key}).as_object().cloned().unwrap();
            let existing = foundry
                .get_documents("settings", Some(&w), None, 0, Some(1))
                .await?;
            let (action, result) = match existing.first() {
                Some(doc) => {
                    let r = foundry.modify_document("Setting", "update", json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{"_id": doc["_id"], "value": serialized}],
                    })).await?;
                    ("updated", r)
                }
                None => {
                    let r = foundry.modify_document("Setting", "create", json!({
                        "action": "create", "broadcast": false, "renderSheet": false, "keepId": false,
                        "data": [{"key": key, "value": serialized}],
                    })).await?;
                    ("created", r)
                }
            };
            Ok(text_response(
                &json!({"action": action, "key": key, "value": value, "result": result}),
            ))
        }
        "list_actor_ownership" | "set_actor_ownership" => {
            let ufields = vec!["_id".into(), "name".into()];
            let users = foundry
                .get_documents("users", None, Some(&ufields), 0, None)
                .await?;
            let uname = |id: &str| {
                users
                    .iter()
                    .find(|u| u["_id"] == json!(id))
                    .and_then(|u| u["name"].as_str().map(String::from))
                    .unwrap_or_else(|| id.to_string())
            };
            let describe = |actor: &Value| {
                let default_map = serde_json::Map::new();
                let ownership = actor
                    .get("ownership")
                    .and_then(Value::as_object)
                    .unwrap_or(&default_map);
                let entries: Vec<Value> = ownership
                    .iter()
                    .filter(|(k, _)| *k != "default")
                    .map(|(uid, lvl)| {
                        json!({
                            "user": uname(uid), "userId": uid,
                            "level": level_name(lvl.as_u64().unwrap_or(0)),
                        })
                    })
                    .collect();
                json!({
                    "_id": actor["_id"], "name": actor["name"],
                    "default": level_name(ownership.get("default").and_then(Value::as_u64).unwrap_or(0)),
                    "users": entries,
                })
            };
            let afields = vec!["_id".into(), "name".into(), "ownership".into()];

            if name == "list_actor_ownership" {
                if let Some(ident) = str_arg(args, "_id").or_else(|| str_arg(args, "name")) {
                    let actor = foundry
                        .find_document("actors", &ident, Some(&afields))
                        .await?
                        .ok_or_else(|| anyhow!("Actor not found"))?;
                    return Ok(text_response(&describe(&actor)));
                }
                let actors = foundry
                    .get_documents("actors", None, Some(&afields), 0, None)
                    .await?;
                let interesting: Vec<Value> = actors
                    .iter()
                    .map(&describe)
                    .filter(|d| {
                        !d["users"].as_array().unwrap().is_empty() || d["default"] != "none"
                    })
                    .collect();
                return Ok(text_response(&Value::Array(interesting)));
            }

            let ident = str_arg(args, "_id")
                .or_else(|| str_arg(args, "name"))
                .ok_or_else(|| anyhow!("Must provide one of: _id or name (actor)"))?;
            let actor = foundry
                .find_document("actors", &ident, Some(&afields))
                .await?
                .ok_or_else(|| anyhow!("Actor not found"))?;
            let mut update = serde_json::Map::new();
            match (str_arg(args, "user"), str_arg(args, "level")) {
                (Some(uarg), Some(level)) => {
                    let user = users
                        .iter()
                        .find(|u| u["_id"] == json!(uarg) || u["name"] == json!(uarg))
                        .ok_or_else(|| anyhow!("User not found: {uarg}"))?;
                    let uid = user["_id"].as_str().unwrap_or("");
                    if level == "none" {
                        update.insert(format!("ownership.-={uid}"), Value::Null);
                    } else {
                        let lvl =
                            level_num(&level).ok_or_else(|| anyhow!("Unknown level: {level}"))?;
                        update.insert(format!("ownership.{uid}"), json!(lvl));
                    }
                }
                (None, None) => {}
                _ => bail!("'user' and 'level' go together"),
            }
            if let Some(dl) = str_arg(args, "default_level") {
                let lvl = level_num(&dl).ok_or_else(|| anyhow!("Unknown level: {dl}"))?;
                update.insert("ownership.default".into(), json!(lvl));
            }
            if update.is_empty() {
                bail!("Nothing to change (provide user+level and/or default_level)");
            }
            let mut update_doc = Value::Object(update.clone());
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
                "applied": update, "result": result,
            })))
        }
        "import_from_compendium" => {
            let pack = str_arg(args, "pack").ok_or_else(|| anyhow!("'pack' is required"))?;
            let doc_type = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            let id = str_arg(args, "_id");
            let dname = str_arg(args, "name");
            if id.is_none() && dname.is_none() {
                bail!("Must provide one of: _id or name");
            }
            let mut query = serde_json::Map::new();
            match (&id, &dname) {
                (Some(i), _) => {
                    query.insert("_id".into(), json!(i));
                }
                (None, Some(n)) => {
                    query.insert("name".into(), json!(n));
                }
                _ => unreachable!(),
            }
            let docs = foundry
                .get_collection(&doc_type, query, false, Some(&pack))
                .await?;
            let mut doc = docs
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("Document not found in pack {pack}"))?;
            doc["folder"] = str_arg(args, "folder")
                .map(|f| json!(f))
                .unwrap_or(Value::Null);
            let keep_id = args
                .get("keep_id")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let imported = json!({"_id": doc["_id"], "name": doc["name"]});
            let result = foundry.modify_document(&doc_type, "create", json!({
                "action": "create", "broadcast": false, "renderSheet": false, "keepId": keep_id,
                "data": [doc],
            })).await?;
            Ok(text_response(
                &json!({"imported": imported, "from": pack, "result": result}),
            ))
        }
        "export_journals" => {
            let where_ = args.get("where").and_then(Value::as_object).cloned();
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            let journals = foundry
                .get_documents("journal", where_.as_ref(), None, 0, None)
                .await?;
            let total = journals.len();
            let page: Vec<Value> = journals.iter().skip(offset).take(limit).map(|j| {
                let empty = vec![];
                let pages: Vec<Value> = j.get("pages").and_then(Value::as_array).unwrap_or(&empty)
                    .iter().map(|p| json!({
                        "name": p["name"],
                        "markdown": html_to_markdown(
                            p.pointer("/text/content").and_then(Value::as_str).unwrap_or("")),
                    })).collect();
                json!({"_id": j["_id"], "name": j["name"], "pages": pages})
            }).collect();
            Ok(text_response(&json!({
                "total": total, "offset": offset, "count": page.len(), "journals": page,
            })))
        }
        "create_compendium" => {
            let label = str_arg(args, "label").ok_or_else(|| anyhow!("'label' is required"))?;
            let doc_type = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            let result = foundry
                .manage_compendium("create", json!({"label": label, "type": doc_type}))
                .await?;
            Ok(text_response(&result))
        }
        "delete_compendium" => {
            let pname = str_arg(args, "name").ok_or_else(|| anyhow!("'name' is required"))?;
            let result = foundry.manage_compendium("delete", json!(pname)).await?;
            Ok(text_response(&result))
        }
        "create_directory" => {
            let target = str_arg(args, "target").ok_or_else(|| anyhow!("'target' is required"))?;
            let source = str_arg(args, "source").unwrap_or_else(|| "data".into());
            let result = foundry
                .manage_files(
                    json!({"action": "createDirectory", "storage": source, "target": target}),
                    json!({}),
                )
                .await?;
            Ok(text_response(&result))
        }
        "browse_files" => {
            let target = str_arg(args, "target").ok_or_else(|| anyhow!("'target' is required"))?;
            let ftype = str_arg(args, "type").unwrap_or_else(|| "image".into());
            let extensions = args.get("extensions").cloned().unwrap_or(json!([
                ".apng", ".avif", ".bmp", ".gif", ".jpeg", ".jpg", ".png", ".svg", ".tiff", ".webp"
            ]));
            let result = foundry.manage_files(
                json!({"action": "browseFiles", "storage": "data", "target": target}),
                json!({"type": ftype, "extensions": extensions, "wildcard": false, "render": true}),
            ).await?;
            Ok(text_response(&result))
        }
        "upload_file" => {
            let target = str_arg(args, "target").ok_or_else(|| anyhow!("'target' is required"))?;
            let filename =
                str_arg(args, "filename").ok_or_else(|| anyhow!("'filename' is required"))?;
            let url = str_arg(args, "url").filter(|s| !s.is_empty());
            let image_data = str_arg(args, "image_data").filter(|s| !s.is_empty());
            let (bytes, content_type) = match (url, image_data) {
                (Some(_), Some(_)) => bail!("Cannot provide both 'url' and 'image_data'"),
                (None, None) => bail!("Must provide either 'url' or 'image_data'"),
                (Some(u), None) => {
                    let resp = foundry.http.get(&u).send().await?;
                    let ct = resp
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("application/octet-stream")
                        .to_string();
                    (resp.bytes().await?.to_vec(), ct)
                }
                (None, Some(b64)) => {
                    let bytes = base64_decode(&b64)?;
                    (bytes, content_type_for(&filename))
                }
            };
            let result = foundry
                .upload_file(&target, &filename, bytes, &content_type)
                .await?;
            Ok(text_response(&result))
        }
        "show_credentials" => {
            let (active, list) = foundry.credentials_info();
            Ok(text_response(
                &json!({"active_index": active, "credentials": list}),
            ))
        }
        "choose_foundry_instance" => {
            let (_, list) = foundry.credentials_info();
            let index = match (
                args.get("item_order").and_then(Value::as_u64),
                str_arg(args, "_id"),
            ) {
                (Some(i), _) => i as usize,
                (None, Some(id)) => list
                    .iter()
                    .position(|c| c["_id"] == json!(id))
                    .ok_or_else(|| anyhow!("Unknown instance _id: {id}"))?,
                _ => bail!("Provide item_order or _id"),
            };
            foundry.choose_instance(index).await?;
            Ok(text_response(
                &json!({"switching_to": index, "note": "reconnexion en cours (~quelques secondes)"}),
            ))
        }
        "wait_for_message" => {
            let where_ = args.get("where").and_then(Value::as_object).cloned();
            let timeout_s = args
                .get("timeout_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(60)
                .min(120);
            let since = args
                .get("since_seq")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| foundry.event_seq());
            let matched = foundry
                .wait_for_event(since, std::time::Duration::from_secs(timeout_s), |e| {
                    if e.event != "modifyDocument" {
                        return false;
                    }
                    let Some(payload) = e
                        .args
                        .iter()
                        .find(|a| a.get("type") == Some(&json!("ChatMessage")))
                    else {
                        return false;
                    };
                    if payload.get("action") != Some(&json!("create")) {
                        return false;
                    }
                    let empty = vec![];
                    let docs = payload
                        .get("result")
                        .and_then(Value::as_array)
                        .unwrap_or(&empty);
                    docs.iter().any(|d| {
                        where_
                            .as_ref()
                            .is_none_or(|w| crate::foundry::documents::matches_where(d, w))
                    })
                })
                .await;
            match matched {
                None => Ok(text_response(
                    &json!({"timeout": true, "waited_seconds": timeout_s, "messages": []}),
                )),
                Some(e) => {
                    let payload = e
                        .args
                        .iter()
                        .find(|a| a.get("type") == Some(&json!("ChatMessage")))
                        .cloned()
                        .unwrap_or(Value::Null);
                    let empty = vec![];
                    let docs = payload
                        .get("result")
                        .and_then(Value::as_array)
                        .unwrap_or(&empty);
                    let hits: Vec<Value> = docs
                        .iter()
                        .filter(|d| {
                            where_
                                .as_ref()
                                .is_none_or(|w| crate::foundry::documents::matches_where(d, w))
                        })
                        .cloned()
                        .collect();
                    Ok(text_response(&json!({"timeout": false, "messages": hits})))
                }
            }
        }
        other => bail!("Unknown tool: {other}"),
    }
}

fn base64_decode(input: &str) -> Result<Vec<u8>> {
    // Décodage base64 standard sans dépendance.
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut rev = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        rev[c as usize] = i as u8;
    }
    let clean: Vec<u8> = input.bytes().filter(|b| !b" \n\r\t".contains(b)).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    for &b in &clean {
        if b == b'=' {
            break;
        }
        let v = rev[b as usize];
        if v == 255 {
            bail!("base64 invalide");
        }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

fn content_type_for(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "json" => "application/json",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        assert_eq!(base64_decode("SGVsbG8=").unwrap(), b"Hello");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a");
        assert!(base64_decode("$$$").is_err());
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type_for("a.PNG"), "image/png");
        assert_eq!(content_type_for("x.bin"), "application/octet-stream");
    }
}
