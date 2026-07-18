//! Transferts entre mondes — le seul endroit qui manipule DEUX connexions
//! Foundry à la fois (lecture sur `from`, écriture sur `to`).
//!
//! Les identifiants sont conservés par défaut : les liens @UUID entre documents
//! copiés continuent de fonctionner dans le monde cible.

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use super::{str_arg, text_response};
use crate::foundry::documents::collection_to_type;
use crate::mcp::McpState;

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("copy_documents",
         "Copy documents from one Foundry instance to another (both must be in FOUNDRY_CREDENTIALS_JSON — see show_credentials). Pick what to copy with `where`/`ids`; _ids are preserved by default so @UUID links between copied documents keep working. Use dry_run first to see what would move. NOTE: image/audio paths are copied as-is — they resolve only if the target server hosts the same files (true for two worlds on the same Foundry install).",
         json!({"type":"object","properties":{
            "from":{"type":"string","description":"source instance _id"},
            "to":{"type":"string","description":"target instance _id"},
            "collection":{"type":"string","description":"actors | items | journals | scenes | tables | macros | playlists | cards | folders"},
            "where":{"type":"object","additionalProperties":true,"description":"filter, same syntax as get_* (dotted paths, __in/__contains/…)"},
            "ids":{"type":"array","items":{"type":"string"},"description":"explicit _ids (wins over where)"},
            "keep_id":{"type":"boolean","description":"preserve _ids so links survive (default true)"},
            "with_folders":{"type":"boolean","description":"also recreate the folders the documents live in (default true)"},
            "overwrite":{"type":"boolean","description":"replace documents that already exist in the target (default false: they are skipped)"},
            "dry_run":{"type":"boolean","description":"report what would be copied without writing anything"},
            "limit":{"type":"number","description":"safety cap (default 200)"}},
            "required":["from","to","collection"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    name == "copy_documents"
}

/// Documents d'une collection sur une instance donnée.
async fn read_from(
    state: &McpState,
    instance: &str,
    collection: &str,
    where_: Option<&Map<String, Value>>,
    ids: Option<&Vec<Value>>,
) -> Result<Vec<Value>> {
    let src = state.resolve(Some(instance)).await?;
    let mut filter = where_.cloned().unwrap_or_default();
    if let Some(ids) = ids {
        filter.insert("_id__in".into(), Value::Array(ids.clone()));
    }
    let filter = if filter.is_empty() {
        None
    } else {
        Some(&filter)
    };
    src.foundry
        .get_documents(collection, filter, None, 0, None)
        .await
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    if name != "copy_documents" {
        bail!("Unknown tool: {name}");
    }
    let from = str_arg(args, "from").ok_or_else(|| anyhow!("'from' is required"))?;
    let to = str_arg(args, "to").ok_or_else(|| anyhow!("'to' is required"))?;
    let collection = str_arg(args, "collection").ok_or_else(|| anyhow!("'collection' required"))?;
    if from == to {
        bail!("'from' et 'to' désignent la même instance ('{from}') — rien à transférer");
    }
    let doc_type = collection_to_type(&collection)
        .ok_or_else(|| anyhow!("collection inconnue : {collection}"))?;
    let keep_id = args.get("keep_id").and_then(Value::as_bool).unwrap_or(true);
    let with_folders = args
        .get("with_folders")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let overwrite = args
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let dry_run = args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(200)
        .max(1) as usize;

    // 1 · lire la source
    let where_ = args.get("where").and_then(Value::as_object);
    let ids = args.get("ids").and_then(Value::as_array);
    let mut docs = read_from(state, &from, &collection, where_, ids).await?;
    let found = docs.len();
    let truncated = found > limit;
    docs.truncate(limit);
    if docs.is_empty() {
        return Ok(text_response(&json!({
            "copied": 0, "reason": "aucun document ne correspond dans la source",
            "from": from, "to": to, "collection": collection,
        })));
    }

    // 2 · ce qui existe déjà dans la cible (par _id, sinon par nom)
    let target = state.resolve(Some(&to)).await?;
    let existing = target
        .foundry
        .get_documents(
            &collection,
            None,
            Some(&["_id".to_string(), "name".to_string()]),
            0,
            None,
        )
        .await?;
    // Un document « déjà là » se reconnaît par son _id quand on les conserve,
    // par son nom sinon. `overwrite` MET À JOUR celui de la cible (en gardant
    // l'_id de la cible) au lieu d'en créer un doublon.
    let twin = |d: &Value| -> Option<Value> {
        existing
            .iter()
            .find(|e| {
                if keep_id {
                    e.get("_id") == d.get("_id")
                } else {
                    e.get("name") == d.get("name")
                }
            })
            .and_then(|e| e.get("_id").cloned())
    };

    let mut to_create = Vec::new();
    let mut to_update = Vec::new();
    let mut skipped = Vec::new();
    for d in &docs {
        match twin(d) {
            Some(target_id) => {
                if !overwrite {
                    skipped.push(d.get("name").cloned().unwrap_or(Value::Null));
                    continue;
                }
                let mut doc = d.clone();
                if let Some(o) = doc.as_object_mut() {
                    o.insert("_id".into(), target_id); // c'est CE document qu'on écrase
                }
                to_update.push(doc);
            }
            None => {
                let mut doc = d.clone();
                if !keep_id && let Some(o) = doc.as_object_mut() {
                    o.remove("_id");
                }
                to_create.push(doc);
            }
        }
    }

    // 3 · dossiers : on recrée ceux dont les documents dépendent
    let mut folders_created = 0usize;
    if with_folders && !(to_create.is_empty() && to_update.is_empty()) && collection != "folders" {
        let needed: Vec<Value> = to_create
            .iter()
            .chain(to_update.iter())
            .filter_map(|d| d.get("folder").cloned())
            .filter(|f| !f.is_null())
            .collect();
        if !needed.is_empty() {
            let mut w = Map::new();
            w.insert("_id__in".into(), Value::Array(needed.clone()));
            let src_folders = read_from(state, &from, "folders", Some(&w), None).await?;
            let target_folders = target
                .foundry
                .get_documents("folders", None, Some(&["_id".to_string()]), 0, None)
                .await?;
            let missing: Vec<Value> = src_folders
                .into_iter()
                .filter(|f| !target_folders.iter().any(|t| t.get("_id") == f.get("_id")))
                .collect();
            folders_created = missing.len();
            if !missing.is_empty() && !dry_run {
                target
                    .foundry
                    .modify_document(
                        "Folder",
                        "create",
                        json!({ "action": "create", "broadcast": false, "renderSheet": false,
                                "keepId": true, "data": missing }),
                    )
                    .await?;
            }
        }
    }

    if dry_run {
        return Ok(text_response(&json!({
            "dry_run": true, "from": from, "to": to, "collection": collection,
            "found": found, "wouldCreate": to_create.len(), "wouldUpdate": to_update.len(),
            "wouldSkipExisting": skipped.len(), "wouldCreateFolders": folders_created,
            "truncated": truncated,
            "sample": to_create.iter().take(5)
                .map(|d| d.get("name").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
        })));
    }

    // 4 · écrire dans la cible
    let mut created = 0usize;
    if !to_create.is_empty() {
        let result = target
            .foundry
            .modify_document(
                doc_type,
                "create",
                json!({ "action": "create", "broadcast": false, "renderSheet": false,
                        "keepId": keep_id, "data": to_create }),
            )
            .await?;
        created = result
            .get("result")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
    }
    let mut updated = 0usize;
    if !to_update.is_empty() {
        target
            .foundry
            .modify_document(
                doc_type,
                "update",
                json!({ "action": "update", "diff": false, "recursive": true,
                        "render": true, "updates": to_update }),
            )
            .await?;
        updated = to_update.len();
    }
    Ok(text_response(&json!({
        "from": from, "to": to, "collection": collection,
        "found": found, "created": created, "updated": updated,
        "skippedExisting": skipped.len(), "foldersCreated": folders_created,
        "keptIds": keep_id, "truncated": truncated,
        "note": "les chemins d'images/sons sont copiés tels quels : ils ne résolvent que si le serveur cible héberge les mêmes fichiers",
    })))
}
