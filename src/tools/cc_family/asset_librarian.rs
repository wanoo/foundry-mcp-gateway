//! Asset Librarian (al_*) — tags flags["asset-librarian"] + navigateur (compagnon).
//!
//! Modèle d'Asset Librarian 1.4.x (relevé dans le code du module) :
//!   · `flags.asset-librarian.categoryTag` : une chaîne ;
//!   · `flags.asset-librarian.filterTag` : un TABLEAU de tags normalisés
//!     (espaces repliés, dédoublonnés sans casse) — l'ancienne forme chaîne
//!     "a,b" reste lisible ;
//!   · réglages monde `tagRegistry` ({[onglet]: {categories, filters}}, le
//!     nuage de tags proposé) et `tagGroupConfig` (groupes de tags) ;
//!   · tags d'images : fichiers Data `asset-librarian/image-tags/shards/xx.json`
//!     ({chemin: {categoryTag, filterTag[]}}).

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use super::mini_calendar::{ensure_module_active, read_setting, write_setting};
use crate::mcp::McpState;
use crate::tools::companion::{Delivery, call_companion, cmd_for};
use crate::tools::{ToolDef, carry, str_arg, text_response};

const MODULE: &str = "asset-librarian";
const SHARD_DIR: &str = "asset-librarian/image-tags/shards";

/// Collections monde que l'on parcourt pour collection = "all".
const TAGGABLE: &[&str] = &[
    "actors",
    "items",
    "journal",
    "scenes",
    "tables",
    "playlists",
    "macros",
    "cards",
];

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

/// Outils `client_*` de ce module → commande compagnon (fusionnés dans
/// `companion::COMPANION_COMMANDS`).
pub const COMMANDS: &[(&str, &str, Delivery)] = &[
    ("client_al_open", "al_open", Delivery::Unique),
    ("client_al_search", "al_search", Delivery::Unique),
];

/// Outils sans effet de bord (annotations + mode FOUNDRY_READONLY).
pub const READ_ONLY: &[&str] = &[
    "al_find",
    "al_list_tags",
    "al_image_tags",
    "client_al_search",
];

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    let tag_list = json!({"oneOf":[{"type":"string"},{"type":"array","items":{"type":"string"}}]});
    vec![
        // --- Asset Librarian ---
        ("al_tag",
         "Asset Librarian: set the tags of a world document — flags.asset-librarian.categoryTag (one string) and filterTag (an array of normalized tags, as the module stores it). category_tag: \"\" clears it. filter_tag replaces the whole list (array, or comma-separated string; \"\" or [] clears); add_filter_tags / remove_filter_tags edit it incrementally (case-insensitive). A legacy \"a,b\" string is converted to an array. New tags are also merged into the tagRegistry setting so they show in the browser's tag cloud (register:false to skip). collection defaults to journal.",
         json!({"type":"object","properties":{
            "_id":{"type":"string"},"name":{"type":"string"},
            "collection":{"type":"string","description":"actors, items, journal (default), scenes, tables, playlists, macros, cards."},
            "category_tag":{"type":"string"},"filter_tag": tag_list,
            "add_filter_tags":{"type":"array","items":{"type":"string"}},
            "remove_filter_tags":{"type":"array","items":{"type":"string"}},
            "register":{"type":"boolean"}}})),
        ("al_find",
         "Asset Librarian: find world documents by Asset Librarian tag — case-insensitive substring match against the category tag and/or each filter tag (exact:true = whole tag only). field: filter (default) | category | any. collection: one world collection (default journal) or \"all\" (actors, items, journal, scenes, tables, playlists, macros, cards). Returns each match with its categoryTag and filterTag list.",
         json!({"type":"object","properties":{
            "tag":{"type":"string"},"field":{"type":"string","enum":["filter","category","any"]},
            "collection":{"type":"string"},"exact":{"type":"boolean"}},"required":["tag"]})),
        ("al_list_tags",
         "Asset Librarian: list the known tags per browser tab (setting tagRegistry: categories and filter tags) and the custom tag groups (setting tagGroupConfig), optionally for one tab (Actor, Item, JournalEntry, Scene, RollTable, Playlist, Macro, Cards, Adventure, Image). The registry is filled by the browser and by al_tag; tags set by other means appear once the browser has loaded that tab.",
         json!({"type":"object","properties":{"tab":{"type":"string"}}})),
        ("al_image_tags",
         "Asset Librarian: read the tags given to image files (Image tab), stored as Data files under asset-librarian/image-tags/shards/*.json. Filter by tag (substring, case-insensitive, category or filter tags), category, or path (substring); limit (default 100). Read-only.",
         json!({"type":"object","properties":{
            "tag":{"type":"string"},"category":{"type":"string"},"path":{"type":"string"},
            "limit":{"type":"integer","minimum":1}}})),
        ("client_al_open",
         "Asset Librarian (companion): open the asset browser on the GM client. mode: world (default) | compendium; tab: Actor, Item (default), JournalEntry, Scene, RollTable, Playlist, Macro, Cards, Adventure, Image (GM, needs image scanning enabled). tags: tag filters (strings, or {value, state}); tag_state: include (any) | and (all) | exclude — default include. search: text in the search box. in: folder name, or a path as an array of folder names. filters: [{key, value, state}] field filters. clear (default true): drop the filters already active.",
         json!({"type":"object","properties":{
            "mode":{"type":"string","enum":["world","compendium"]},"tab":{"type":"string"},
            "tags":{"type":"array","items":{"oneOf":[{"type":"string"},{"type":"object","properties":{"value":{"type":"string"},"state":{"type":"string","enum":["include","and","exclude"]}}}]}},
            "tag_state":{"type":"string","enum":["include","and","exclude"]},
            "search":{"type":"string"},
            "in":{"oneOf":[{"type":"string"},{"type":"array","items":{"type":"string"}}]},
            "clear":{"type":"boolean"},
            "filters":{"type":"array","items":{"type":"object","additionalProperties":true}}}})),
        ("client_al_search",
         "Asset Librarian (companion): search assets the way the browser lists them (DataManager.getAssets) — world documents or the compendiums enabled in Asset Librarian (mode). type: Actor, Item (default), JournalEntry, Scene, RollTable, Playlist, Macro, Cards, Adventure. tags + tag_match (any | all) filter on filter/category tags (case-insensitive, whole tag); category: category tag; query: name substring. Returns name, uuid, type, img, folder and tags; limit default 50.",
         json!({"type":"object","properties":{
            "type":{"type":"string"},"mode":{"type":"string","enum":["world","compendium"]},
            "tags":{"type":"array","items":{"type":"string"}},
            "tag_match":{"type":"string","enum":["any","all"]},
            "category":{"type":"string"},"query":{"type":"string"},
            "limit":{"type":"integer","minimum":1}}})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

// ------------------------------------------------------------- logique pure

/// normalizeTag du module : espaces de bord retirés, internes repliés.
fn normalize_tag(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// normalizeTagList : tableau ou chaîne "a,b", vides retirés, dédoublonné
/// sans tenir compte de la casse (la première graphie gagne).
fn normalize_tag_list(raw: Option<&Value>) -> Vec<String> {
    let entries: Vec<String> = match raw {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        Some(Value::String(s)) => s.split(',').map(String::from).collect(),
        Some(other) => vec![other.to_string()],
    };
    let mut out: Vec<String> = Vec::new();
    for e in entries {
        let n = normalize_tag(&e);
        if n.is_empty() || out.iter().any(|x| x.to_lowercase() == n.to_lowercase()) {
            continue;
        }
        out.push(n);
    }
    out
}

/// Tags après application des arguments d'al_tag : (catégorie, filtres,
/// catégorie touchée, filtres touchés).
fn next_tags(
    cur_cat: &str,
    cur_filters: Option<&Value>,
    args: &Value,
) -> Result<(String, Vec<String>, bool, bool)> {
    let mut cat = normalize_tag(cur_cat);
    let mut filters = normalize_tag_list(cur_filters);
    let mut cat_touched = false;
    let mut filt_touched = false;
    if let Some(c) = args.get("category_tag") {
        let c = c
            .as_str()
            .ok_or_else(|| anyhow!("category_tag must be a string"))?;
        cat = normalize_tag(c);
        cat_touched = true;
    }
    if let Some(f) = args.get("filter_tag") {
        filters = normalize_tag_list(Some(f));
        filt_touched = true;
    }
    if let Some(add) = args.get("add_filter_tags") {
        let mut all = filters.iter().map(|s| json!(s)).collect::<Vec<_>>();
        all.extend(normalize_tag_list(Some(add)).into_iter().map(Value::from));
        filters = normalize_tag_list(Some(&Value::Array(all)));
        filt_touched = true;
    }
    if let Some(rm) = args.get("remove_filter_tags") {
        let rm: Vec<String> = normalize_tag_list(Some(rm))
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        filters.retain(|t| !rm.contains(&t.to_lowercase()));
        filt_touched = true;
    }
    // Une ancienne valeur chaîne est réécrite en tableau même sans changement.
    if matches!(cur_filters, Some(Value::String(_))) {
        filt_touched = true;
    }
    if !cat_touched && !filt_touched {
        bail!("Provide category_tag, filter_tag, add_filter_tags and/or remove_filter_tags");
    }
    Ok((cat, filters, cat_touched, filt_touched))
}

/// Diff Foundry pour ces tags (clé `-=` = suppression du flag, comme unsetFlag).
fn tag_update(
    cat: &str,
    filters: &[String],
    cat_touched: bool,
    filt_touched: bool,
) -> Map<String, Value> {
    let mut u = Map::new();
    if cat_touched {
        if cat.is_empty() {
            u.insert(format!("flags.{MODULE}.-=categoryTag"), Value::Null);
        } else {
            u.insert(format!("flags.{MODULE}.categoryTag"), json!(cat));
        }
    }
    if filt_touched {
        if filters.is_empty() {
            u.insert(format!("flags.{MODULE}.-=filterTag"), Value::Null);
        } else {
            u.insert(format!("flags.{MODULE}.filterTag"), json!(filters));
        }
    }
    u
}

/// Un tag correspond-il à la recherche (sous-chaîne ou exact, sans casse) ?
fn tag_matches(tag: &str, needle: &str, exact: bool) -> bool {
    let (t, n) = (tag.to_lowercase(), needle.trim().to_lowercase());
    if exact { t == n } else { t.contains(&n) }
}

/// Onglet Asset Librarian d'une collection monde.
fn tab_of(collection: &str) -> Option<&'static str> {
    crate::foundry::documents::collection_to_type(collection).filter(|t| {
        [
            "Actor",
            "Item",
            "JournalEntry",
            "Scene",
            "RollTable",
            "Playlist",
            "Macro",
            "Cards",
        ]
        .contains(t)
    })
}

/// Fusionne des tags dans tagRegistry[tab] ; None si rien ne change.
fn merge_registry(registry: &Value, tab: &str, cat: &str, filters: &[String]) -> Option<Value> {
    let mut reg = registry.as_object().cloned().unwrap_or_default();
    let entry = reg.get(tab).cloned().unwrap_or(json!({}));
    let mut cats: Vec<Value> = entry
        .get("categories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !cat.is_empty() {
        cats.push(json!(cat));
    }
    let mut fl: Vec<Value> = entry
        .get("filters")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    fl.extend(filters.iter().map(|f| json!(f)));
    let cats = normalize_tag_list(Some(&Value::Array(cats)));
    let fl = normalize_tag_list(Some(&Value::Array(fl)));
    let next = json!({"categories": cats, "filters": fl});
    let same = normalize_tag_list(entry.get("categories")) == cats
        && normalize_tag_list(entry.get("filters")) == fl
        && entry.get("categories").is_some()
        && entry.get("filters").is_some();
    if same {
        return None;
    }
    reg.insert(tab.into(), next);
    Some(Value::Object(reg))
}

/// Tags d'un document : (catégorie, filtres).
fn doc_tags(doc: &Value) -> (String, Vec<String>) {
    let f = doc.pointer(&format!("/flags/{MODULE}"));
    (
        f.and_then(|f| f.get("categoryTag"))
            .and_then(Value::as_str)
            .map(normalize_tag)
            .unwrap_or_default(),
        normalize_tag_list(f.and_then(|f| f.get("filterTag"))),
    )
}

/// Groupes de tags d'un onglet, dans l'ordre persistant puis alphabétique.
fn ordered_groups(section: &Value) -> Vec<Value> {
    let groups = section
        .get("groups")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut ids: Vec<String> = section
        .get("groupOrder")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|g| groups.contains_key(*g))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    ids.dedup();
    let label = |id: &str| {
        groups[id]
            .get("label")
            .and_then(Value::as_str)
            .map(normalize_tag)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| id.to_string())
    };
    let mut rest: Vec<String> = groups
        .keys()
        .filter(|k| !ids.contains(k))
        .cloned()
        .collect();
    rest.sort_by_key(|id| label(id).to_lowercase());
    ids.extend(rest);
    ids.iter()
        .map(|id| {
            let tags = groups[id].get("tags").cloned().unwrap_or(json!({}));
            json!({"id": id, "label": label(id), "tags": tags})
        })
        .collect()
}

// ---------------------------------------------------------------------- run

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        // ---------------------------------------------------- Asset Librarian
        "al_tag" => {
            let collection = str_arg(args, "collection").unwrap_or_else(|| "journal".into());
            let doc_type = crate::foundry::documents::collection_to_type(&collection)
                .ok_or_else(|| anyhow!("Unknown collection: {collection}"))?;
            let ident = str_arg(args, "_id")
                .or_else(|| str_arg(args, "name"))
                .ok_or_else(|| anyhow!("Must provide _id or name"))?;
            let doc = foundry
                .find_document(
                    &collection,
                    &ident,
                    Some(&["_id".into(), "name".into(), "flags".into()]),
                )
                .await?
                .ok_or_else(|| anyhow!("Document not found: {ident}"))?;
            let flags = doc.pointer(&format!("/flags/{MODULE}"));
            let cur_cat = flags
                .and_then(|f| f.get("categoryTag"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let (cat, filters, ct, ft) =
                next_tags(cur_cat, flags.and_then(|f| f.get("filterTag")), args)?;
            let mut update = tag_update(&cat, &filters, ct, ft);
            update.insert("_id".into(), doc["_id"].clone());
            let result = foundry
                .modify_document(
                    doc_type,
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [Value::Object(update)],
                    }),
                )
                .await?;
            let mut registered = Value::Null;
            if args
                .get("register")
                .and_then(Value::as_bool)
                .unwrap_or(true)
                && let Some(tab) = tab_of(&collection)
                && (!cat.is_empty() || !filters.is_empty())
            {
                let reg = read_setting(foundry, &format!("{MODULE}.tagRegistry"))
                    .await?
                    .unwrap_or(json!({}));
                registered = match merge_registry(&reg, tab, &cat, &filters) {
                    Some(next) => {
                        write_setting(foundry, &format!("{MODULE}.tagRegistry"), &next).await?;
                        json!({"tab": tab, "updated": true})
                    }
                    None => json!({"tab": tab, "updated": false}),
                };
            }
            Ok(text_response(&json!({
                "document": {"_id": doc["_id"], "name": doc["name"]},
                "tags": {"categoryTag": cat, "filterTag": filters},
                "registry": registered, "result": result,
            })))
        }
        "al_find" => {
            let tag = str_arg(args, "tag").ok_or_else(|| anyhow!("'tag' is required"))?;
            if tag.trim().is_empty() {
                bail!("'tag' cannot be empty");
            }
            let collection = str_arg(args, "collection").unwrap_or_else(|| "journal".into());
            let field = str_arg(args, "field").unwrap_or_else(|| "filter".into());
            if !["filter", "category", "any"].contains(&field.as_str()) {
                bail!("field must be filter, category or any");
            }
            let exact = args.get("exact").and_then(Value::as_bool).unwrap_or(false);
            let collections: Vec<String> = if collection == "all" {
                TAGGABLE.iter().map(|s| s.to_string()).collect()
            } else {
                if crate::foundry::documents::collection_to_type(&collection).is_none() {
                    bail!("Unknown collection: {collection}");
                }
                vec![collection.clone()]
            };
            let fields: Vec<String> = vec!["_id".into(), "name".into(), "flags".into()];
            let mut matches = Vec::new();
            for coll in &collections {
                let docs = foundry
                    .get_documents(coll, None, Some(&fields), 0, None)
                    .await?;
                let ty = crate::foundry::documents::collection_to_type(coll).unwrap_or_default();
                for d in docs {
                    let (cat, filters) = doc_tags(&d);
                    let hit_cat =
                        field != "filter" && !cat.is_empty() && tag_matches(&cat, &tag, exact);
                    let hit_filter =
                        field != "category" && filters.iter().any(|f| tag_matches(f, &tag, exact));
                    if hit_cat || hit_filter {
                        matches.push(json!({
                            "collection": coll, "_id": d["_id"], "name": d["name"],
                            "uuid": format!("{ty}.{}", d["_id"].as_str().unwrap_or_default()),
                            "categoryTag": cat, "filterTag": filters,
                        }));
                    }
                }
            }
            Ok(text_response(&json!({
                "field": field, "tag": tag, "exact": exact, "collections": collections,
                "count": matches.len(), "matches": matches,
            })))
        }
        "al_list_tags" => {
            ensure_module_active(foundry, MODULE, "Asset Librarian").await?;
            let tab = str_arg(args, "tab");
            let registry = read_setting(foundry, &format!("{MODULE}.tagRegistry"))
                .await?
                .unwrap_or(json!({}));
            let groups_cfg = read_setting(foundry, &format!("{MODULE}.tagGroupConfig"))
                .await?
                .unwrap_or(json!({}));
            let mut tabs: Vec<String> = registry
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            for k in groups_cfg
                .as_object()
                .map(|o| o.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
            {
                if !tabs.contains(&k) {
                    tabs.push(k);
                }
            }
            if let Some(t) = &tab {
                tabs.retain(|x| x.eq_ignore_ascii_case(t));
            }
            tabs.sort();
            let out: Vec<Value> = tabs
                .iter()
                .map(|t| {
                    let e = registry.get(t);
                    json!({
                        "tab": t,
                        "categories": normalize_tag_list(e.and_then(|e| e.get("categories"))),
                        "filters": normalize_tag_list(e.and_then(|e| e.get("filters"))),
                        "groups": groups_cfg.get(t).map(ordered_groups).unwrap_or_default(),
                    })
                })
                .collect();
            Ok(text_response(&json!({"tabs": out, "count": out.len()})))
        }
        "al_image_tags" => {
            ensure_module_active(foundry, MODULE, "Asset Librarian").await?;
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .max(1) as usize;
            let tag = str_arg(args, "tag").filter(|s| !s.trim().is_empty());
            let category = str_arg(args, "category").filter(|s| !s.trim().is_empty());
            let path = str_arg(args, "path").filter(|s| !s.trim().is_empty());
            let listing = foundry
                .manage_files(
                    json!({"action": "browseFiles", "storage": "data", "target": SHARD_DIR}),
                    json!({"type": "text", "extensions": [".json"], "wildcard": false, "render": true}),
                )
                .await;
            let files: Vec<String> = match listing {
                Ok(l) => l
                    .get("files")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default(),
                Err(_) => vec![],
            };
            let hostname = foundry.hostname();
            let (host, base) = crate::foundry::auth::split_host(&hostname);
            let (http_s, _) = crate::foundry::auth::schemes(&hostname);
            let mut entries = Vec::new();
            let mut errors = Vec::new();
            let mut total = 0usize;
            for f in &files {
                let url = format!("{http_s}://{host}{base}/{}", f.trim_start_matches('/'));
                let map: Value = match foundry.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => r.json().await.unwrap_or(json!({})),
                    Ok(r) => {
                        errors.push(json!({"file": f, "error": format!("HTTP {}", r.status())}));
                        continue;
                    }
                    Err(e) => {
                        errors.push(json!({"file": f, "error": e.to_string()}));
                        continue;
                    }
                };
                for (img, e) in map.as_object().cloned().unwrap_or_default() {
                    let (cat, filters) = doc_tags(&json!({"flags": {MODULE: e}}));
                    if cat.is_empty() && filters.is_empty() {
                        continue;
                    }
                    total += 1;
                    if path
                        .as_ref()
                        .is_some_and(|p| !img.to_lowercase().contains(&p.to_lowercase()))
                    {
                        continue;
                    }
                    if category
                        .as_ref()
                        .is_some_and(|c| !tag_matches(&cat, c, false))
                    {
                        continue;
                    }
                    if tag.as_ref().is_some_and(|t| {
                        !tag_matches(&cat, t, false)
                            && !filters.iter().any(|x| tag_matches(x, t, false))
                    }) {
                        continue;
                    }
                    entries.push(json!({"path": img, "categoryTag": cat, "filterTag": filters}));
                }
            }
            entries.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
            let matched = entries.len();
            entries.truncate(limit);
            let mut out = json!({
                "shards": files.len(), "taggedImages": total, "matched": matched,
                "truncated": matched > limit, "images": entries,
            });
            if files.is_empty() {
                out["note"] = json!(
                    "No image tag files found (asset-librarian/image-tags/shards): no image has been tagged yet."
                );
            }
            if !errors.is_empty() {
                out["errors"] = json!(errors);
            }
            Ok(text_response(&out))
        }
        "client_al_open" => {
            let mut a = json!({
                "mode": str_arg(args, "mode").unwrap_or_else(|| "world".into()),
                "tab": str_arg(args, "tab").unwrap_or_else(|| "Item".into()),
            });
            carry(
                args,
                &["filters", "tags", "tag_state", "search", "in", "clear"],
                &mut a,
            );
            let r = call_companion(state, cmd_for(name), a, None, 10).await?;
            Ok(text_response(&r))
        }
        "client_al_search" => {
            let mut a = json!({
                "type": str_arg(args, "type").unwrap_or_else(|| "Item".into()),
                "mode": str_arg(args, "mode").unwrap_or_else(|| "world".into()),
            });
            carry(
                args,
                &["tags", "tag_match", "category", "query", "limit"],
                &mut a,
            );
            let r = call_companion(state, cmd_for(name), a, None, 30).await?;
            Ok(text_response(&r))
        }

        other => bail!("Unknown tool: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_des_tags() {
        assert_eq!(normalize_tag("  Grande   ville "), "Grande ville");
        assert_eq!(
            normalize_tag_list(Some(&json!("a, b ,A,,c"))),
            vec!["a", "b", "c"]
        );
        assert_eq!(
            normalize_tag_list(Some(&json!(["Port", "port", " Marché  noir"]))),
            vec!["Port", "Marché noir"]
        );
        assert!(normalize_tag_list(None).is_empty());
    }

    #[test]
    fn calcul_des_tags() {
        let (c, f, ct, ft) = next_tags(
            "Lieu",
            Some(&json!("a,b")),
            &json!({"add_filter_tags": ["C", "a"]}),
        )
        .unwrap();
        assert_eq!((c.as_str(), ct, ft), ("Lieu", false, true));
        assert_eq!(f, vec!["a", "b", "C"]);
        let (_, f, _, _) = next_tags(
            "",
            Some(&json!(["a", "B"])),
            &json!({"remove_filter_tags": ["b"]}),
        )
        .unwrap();
        assert_eq!(f, vec!["a"]);
        let (c, f, ct, ft) = next_tags(
            "X",
            Some(&json!(["a"])),
            &json!({"category_tag": "", "filter_tag": ""}),
        )
        .unwrap();
        assert!(c.is_empty() && f.is_empty() && ct && ft);
        // Chaîne héritée réécrite en tableau même sans autre changement.
        let (_, f, _, ft) =
            next_tags("", Some(&json!("x,y")), &json!({"category_tag": "K"})).unwrap();
        assert!(ft);
        assert_eq!(f, vec!["x", "y"]);
        assert!(next_tags("", None, &json!({})).is_err());
    }

    #[test]
    fn diff_foundry() {
        let u = tag_update("", &[], true, true);
        assert_eq!(
            u.get("flags.asset-librarian.-=categoryTag"),
            Some(&Value::Null)
        );
        assert_eq!(
            u.get("flags.asset-librarian.-=filterTag"),
            Some(&Value::Null)
        );
        let u = tag_update("Lieu", &["a".into()], true, true);
        assert_eq!(u["flags.asset-librarian.categoryTag"], "Lieu");
        assert_eq!(u["flags.asset-librarian.filterTag"], json!(["a"]));
        assert!(tag_update("x", &[], false, false).is_empty());
    }

    #[test]
    fn correspondance_et_registre() {
        assert!(tag_matches("Marché noir", "marché", false));
        assert!(!tag_matches("Marché noir", "marché", true));
        assert!(tag_matches("Port", "PORT", true));
        let reg = json!({"Item": {"categories": ["Arme"], "filters": ["rare"]}});
        let next = merge_registry(&reg, "JournalEntry", "Lieu", &["port".into()]).unwrap();
        assert_eq!(
            next["JournalEntry"],
            json!({"categories": ["Lieu"], "filters": ["port"]})
        );
        assert_eq!(next["Item"], reg["Item"]);
        assert!(merge_registry(&reg, "Item", "arme", &["Rare".into()]).is_none());
        assert_eq!(tab_of("journal"), Some("JournalEntry"));
        assert_eq!(tab_of("users"), None);
    }

    #[test]
    fn groupes_ordonnes() {
        let section = json!({"groups": {"g2": {"label": "Zèbre"}, "g1": {"label": "Alpha", "tags": {"a": true}}, "g3": {"label": "Beta"}},
                             "groupOrder": ["g2", "missing"]});
        let ids: Vec<String> = ordered_groups(&section)
            .iter()
            .map(|g| g["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, vec!["g2", "g1", "g3"]);
    }
}
