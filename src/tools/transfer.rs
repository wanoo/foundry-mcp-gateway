//! Transferts entre mondes — le seul endroit qui manipule DEUX connexions
//! Foundry à la fois (lecture sur `from`, écriture sur `to`).
//!
//! Les identifiants sont conservés par défaut : les liens @UUID entre documents
//! copiés continuent de fonctionner dans le monde cible.

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use super::{plural_to_collection, str_arg, text_response};
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
        ("copy_assets",
         "Copy files (maps, tokens, art, audio) from one Foundry instance's storage to another — the companion piece to copy_documents when the two worlds live on DIFFERENT servers, otherwise images end up broken. Walks a source directory (recursively by default), recreates the directory tree on the target and uploads what is missing. Existing files are skipped unless overwrite. Start with dry_run.",
         json!({"type":"object","properties":{
            "from":{"type":"string","description":"source instance _id"},
            "to":{"type":"string","description":"target instance _id"},
            "source_dir":{"type":"string","description":"e.g. worlds/star-wars/assets"},
            "target_dir":{"type":"string","description":"destination path (default: same as source_dir)"},
            "recursive":{"type":"boolean","description":"walk sub-directories (default true)"},
            "extensions":{"type":"array","items":{"type":"string"},"description":"restrict to these, e.g. [\".webp\",\".png\"]"},
            "overwrite":{"type":"boolean","description":"re-upload files already present in the target (default false)"},
            "dry_run":{"type":"boolean"},
            "limit":{"type":"number","description":"safety cap on files (default 100)"}},
            "required":["from","to","source_dir"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(name, "copy_documents" | "copy_assets")
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

/// Contenu d'un répertoire de stockage sur une instance.
async fn browse(
    state: &McpState,
    dir: &str,
    extensions: &Value,
) -> Result<(Vec<String>, Vec<String>)> {
    let r = state
        .foundry
        .manage_files(
            json!({"action": "browseFiles", "storage": "data", "target": dir}),
            json!({"type": "image", "extensions": extensions, "wildcard": false, "render": false}),
        )
        .await?;
    let take = |k: &str| -> Vec<String> {
        r.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok((take("dirs"), take("files")))
}

/// Toutes les extensions gérées par Foundry (images, audio, vidéo) — un asset
/// de scène peut être un .webm ou un .ogg autant qu'un .webp.
fn all_extensions() -> Value {
    json!([
        ".apng", ".avif", ".bmp", ".gif", ".jpeg", ".jpg", ".png", ".svg", ".tiff", ".webp",
        ".aac", ".flac", ".m4a", ".mid", ".mp3", ".ogg", ".opus", ".wav", ".webm", ".mp4", ".ogv",
        ".json", ".pdf", ".txt", ".md"
    ])
}

async fn copy_assets(state: &McpState, args: &Value) -> Result<Value> {
    let from = str_arg(args, "from").ok_or_else(|| anyhow!("'from' is required"))?;
    let to = str_arg(args, "to").ok_or_else(|| anyhow!("'to' is required"))?;
    if from == to {
        bail!("'from' et 'to' désignent la même instance ('{from}')");
    }
    let source_dir = str_arg(args, "source_dir").ok_or_else(|| anyhow!("'source_dir' required"))?;
    let target_dir = str_arg(args, "target_dir").unwrap_or_else(|| source_dir.clone());
    let recursive = args
        .get("recursive")
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
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
    let extensions = args
        .get("extensions")
        .cloned()
        .unwrap_or_else(all_extensions);

    let src = state.resolve(Some(&from)).await?;
    let dst = state.resolve(Some(&to)).await?;

    // 1 · parcourir la source (largeur d'abord, pour un rapport lisible)
    let mut queue = vec![source_dir.clone()];
    let mut files: Vec<String> = Vec::new();
    let mut dirs: Vec<String> = vec![source_dir.clone()];
    while let Some(dir) = queue.pop() {
        let (sub, found) = browse(&src, &dir, &extensions).await?;
        files.extend(found);
        if recursive {
            for d in sub {
                dirs.push(d.clone());
                queue.push(d);
            }
        }
        if files.len() > limit * 4 {
            break; // garde-fou : arborescence démesurée
        }
    }
    let found_total = files.len();
    let truncated = found_total > limit;
    files.truncate(limit);

    // 2 · ce qui manque côté cible
    let rebase = |p: &str| -> String { p.replacen(&source_dir, &target_dir, 1) };
    let mut missing = Vec::new();
    for f in &files {
        let dest = rebase(f);
        let parent = dest
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();
        let (_, present) = browse(&dst, &parent, &extensions).await.unwrap_or_default();
        if overwrite || !present.contains(&dest) {
            missing.push((f.clone(), dest));
        }
    }

    if dry_run {
        return Ok(text_response(&json!({
            "dry_run": true, "from": from, "to": to,
            "sourceDir": source_dir, "targetDir": target_dir,
            "directories": dirs.len(), "filesFound": found_total,
            "wouldUpload": missing.len(), "truncated": truncated,
            "sample": missing.iter().take(5).map(|(s, _)| s).collect::<Vec<_>>(),
        })));
    }

    // 3 · créer l'arborescence cible puis transférer, fichier par fichier
    for d in &dirs {
        let _ = dst
            .foundry
            .manage_files(
                json!({"action": "createDirectory", "storage": "data", "target": rebase(d)}),
                json!({}),
            )
            .await; // déjà existant = pas une erreur pour nous
    }
    let src_hostname = src.foundry.hostname();
    let (host, base) = crate::foundry::auth::split_host(&src_hostname);
    let (http_s, _) = crate::foundry::auth::schemes(&src_hostname);
    let mut uploaded = 0usize;
    let mut failed = Vec::new();
    for (source_path, dest_path) in &missing {
        let url = format!("{http_s}://{host}{base}/{source_path}");
        let dir = dest_path
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();
        let filename = dest_path.rsplit('/').next().unwrap_or_default().to_string();
        // On rapatrie via la passerelle : le serveur cible n'a pas besoin
        // d'atteindre le serveur source (ils peuvent être sur deux réseaux).
        match src.foundry.http.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let ct = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("application/octet-stream")
                    .to_string();
                match resp.bytes().await {
                    Ok(bytes) => match dst
                        .foundry
                        .upload_file(&dir, &filename, bytes.to_vec(), &ct)
                        .await
                    {
                        Ok(_) => uploaded += 1,
                        Err(e) => failed.push(json!({"file": source_path, "error": e.to_string()})),
                    },
                    Err(e) => failed.push(json!({"file": source_path, "error": e.to_string()})),
                }
            }
            Ok(resp) => failed
                .push(json!({"file": source_path, "error": format!("HTTP {}", resp.status())})),
            Err(e) => failed.push(json!({"file": source_path, "error": e.to_string()})),
        }
    }
    Ok(text_response(&json!({
        "from": from, "to": to, "sourceDir": source_dir, "targetDir": target_dir,
        "filesFound": found_total, "uploaded": uploaded,
        "skippedExisting": files.len() - missing.len(),
        "failed": failed, "truncated": truncated,
    })))
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    if name == "copy_assets" {
        return copy_assets(state, args).await;
    }
    if name != "copy_documents" {
        bail!("Unknown tool: {name}");
    }
    let from = str_arg(args, "from").ok_or_else(|| anyhow!("'from' is required"))?;
    let to = str_arg(args, "to").ok_or_else(|| anyhow!("'to' is required"))?;
    // « journals » (comme les outils get_*) autant que « journal » (nom interne).
    let collection = str_arg(args, "collection")
        .map(|c| plural_to_collection(&c).to_string())
        .ok_or_else(|| anyhow!("'collection' required"))?;
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
