//! Mass Edit — presets (me_*) : des préfabriqués (lumières, tuiles, murs…)
//! rangés dans des compendiums et posés sur une scène.
//!
//! Modèle de Mass Edit 3.3.x (relevé dans le code du module) :
//!   · un preset = un JournalEntry d'un compendium « géré », reconnaissable à
//!     son document `MassEditMetaData` dont flags["multi-token-edit"].index
//!     vaut {id: {img, documentName, tags}} ; les données (placeables, scripts)
//!     sont dans flags["multi-token-edit"].preset du JournalEntry ;
//!   · cet index n'est tenu à jour QUE par le client qui écrit : la création et
//!     la pose passent donc par le compagnon (navigateur du MJ) ; la passerelle
//!     se contente de lire (me_list_presets) ;
//!   · `preSpawnScript` / `postSpawnScript` sont du JavaScript exécuté au
//!     spawn : le compagnon les soumet au réglage « Remote script execution »,
//!     et le garde-fou de la passerelle refuse d'écrire de tels presets.

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::mcp::McpState;
use crate::tools::ToolDef;
use crate::tools::companion::{Delivery, call_companion, cmd_for};
use crate::tools::{carry, str_arg, text_response};

const MODULE: &str = "multi-token-edit";
const META_INDEX_ID: &str = "MassEditMetaData";

#[rustfmt::skip]
const PIVOTS: &[&str] = &[
    "TOP_LEFT", "TOP", "TOP_RIGHT", "LEFT", "CENTER", "RIGHT",
    "BOTTOM_LEFT", "BOTTOM", "BOTTOM_RIGHT",
];

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

pub const COMMANDS: &[(&str, &str, Delivery)] = &[
    (
        "client_me_find_presets",
        "me_find_presets",
        Delivery::Unique,
    ),
    (
        "client_me_spawn_preset",
        "me_spawn_preset",
        Delivery::Unique,
    ),
    (
        "client_me_create_preset",
        "me_create_preset",
        Delivery::Unique,
    ),
];

pub const READ_ONLY: &[&str] = &["me_list_presets", "client_me_find_presets"];

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    let tags = json!({"oneOf":[{"type":"string"},{"type":"array","items":{"type":"string"}}],
        "description":"Tags (array, or comma-separated string); a preset must carry ALL of them."});
    let types = json!({"type":"array","items":{"type":"string"},
        "description":"Preset types: Token, Tile, Drawing, Wall, AmbientLight, AmbientSound, Note, Region (also Scene, Bag)."});
    let query = json!({"type":"string",
        "description":"Mass Edit search syntax: words = case-insensitive name substring (\"quoted phrase\" for one term), #tag, @Type (case-sensitive, e.g. @AmbientLight), -word / -#tag / -@Type to exclude, #null = no tags, AND / OR (implicit AND between terms) and parentheses. Cannot be combined with name/tags/types."});
    vec![
        ("me_list_presets",
         "Mass Edit: list presets server-side, without the GM browser, by reading the MassEditMetaData index of every preset compendium (or only `pack`). Returns uuid, name, documentName, tags, img and pack of each preset; optional filters name (substring, case-insensitive), tag (exact, case-insensitive) and type (documentName). It does NOT load the presets, so it cannot tell whether they carry spawn scripts or active tiles — use client_me_find_presets for that. Scans all JournalEntry packs when `pack` is omitted (one request per pack). limit default 200.",
         json!({"type":"object","properties":{
            "pack":{"type":"string","description":"Compendium collection id, e.g. world.mass-edit-presets-main."},
            "name":{"type":"string"},"tag":{"type":"string"},"type":{"type":"string"},
            "limit":{"type":"integer","minimum":1}}})),
        ("client_me_find_presets",
         "Mass Edit (companion): search the presets (prefabs to place on a scene) of every preset compendium, the way the Preset Browser does (MassEdit.getPresets). Either `query` (module search syntax) OR name/tags/types (combined with AND; name matches as one case-insensitive substring) — never both: Mass Edit rejects a query mixed with name/tags/types. uuid fetches one preset directly. include_files also searches the indexed file directories (virtual image/audio presets). Returns per preset: uuid, name, documentName, tags, img, pack, elements (placeables in the preset), attached count, hasScripts (non-empty pre/post-spawn script — placing it needs the GM's \"Remote script execution\" setting) and hasActiveTiles (contains Monk's Active Tiles actions, checked again at spawn). Sorted by name; limit default 50 (max 500), `total` gives the full match count.",
         json!({"type":"object","properties":{
            "uuid":{"type":"string"},"query":query,"name":{"type":"string"},"tags":tags,"types":types,
            "include_files":{"type":"boolean"},"limit":{"type":"integer","minimum":1,"maximum":500}}})),
        ("client_me_spawn_preset",
         "Mass Edit (companion): place a preset on the scene the GM is currently VIEWING, at canvas pixel x,y (both required; the preset's bounding box is aligned on x,y by `pivot`, default TOP_LEFT). Pick it by uuid, or by query / name / tags / types (same rules as client_me_find_presets): several matches are an error listing them unless random:true (an exact name match wins). `scene` (id or name) is only a safety check: Mass Edit snaps and resolves levels against the displayed canvas, so a scene other than the viewed one is refused — view it first (activate_scene / pull_users_to_scene). hidden: spawn hidden; snap_to_grid (default true; a GM holding Shift at that moment disables it); scale_to_grid: rescale Tiles/Drawings/Walls to this scene's grid. Never opens the module's prompts (modifyPrompt off, no preview). Refused unless the GM allowed remote script execution (\"Remote script execution\" setting; \"confirm\" asks the GM): presets with a pre/post-spawn script, with Monk's Active Tiles actions outside the safe list or evaluating values, or with executeScript/executeMacro region behaviors or Item Piles macros (macro setting). Returns the created documents (documentName, id, name, x, y, hidden).",
         json!({"type":"object","properties":{
            "uuid":{"type":"string"},"query":query,"name":{"type":"string"},"tags":tags,"types":types,
            "random":{"type":"boolean","description":"Pick one at random when several presets match."},
            "x":{"type":"number"},"y":{"type":"number"},
            "scene":{"type":"string","description":"Expected scene (id or name): must be the one the GM views."},
            "hidden":{"type":"boolean"},"snap_to_grid":{"type":"boolean"},"scale_to_grid":{"type":"boolean"},
            "pivot":{"type":"string","enum":PIVOTS}},
            "required":["x","y"]})),
        ("client_me_create_preset",
         "Mass Edit (companion): save placeables of the scene the GM is VIEWING as a new preset (MassEdit.createPresetFromPlaceable): document_name + ids are the preset's main placeables (one type), attached [{document_name, id}] adds placeables of other types spawned with them (e.g. a lamp tile + its light). name required; tags (slugified, as the module does); img (thumbnail, default the placeable's texture or type icon). Stored in `pack` or Mass Edit's working pack; when that is the default world.mass-edit-presets-main and it does not exist yet, it is created exactly as the Preset Browser does on first use (packCreated:true) — any other missing or non-preset pack is an error. Refused when the placeables can run code (Monk's Active Tiles actions outside the safe list, executeScript/executeMacro region behaviors, Item Piles macros): every later spawn would run it. Spawn scripts cannot be set here. Returns the preset summary (uuid, name, tags, elements, attached) and whether the MassEditMetaData index was updated.",
         json!({"type":"object","properties":{
            "document_name":{"type":"string","description":"Token, Tile, Drawing, Wall, AmbientLight, AmbientSound, Note or Region."},
            "ids":{"type":"array","items":{"type":"string"}},
            "attached":{"type":"array","items":{"type":"object","properties":{
                "document_name":{"type":"string"},"id":{"type":"string"}},"required":["document_name","id"]}},
            "name":{"type":"string"},"tags":tags,"img":{"type":"string"},
            "pack":{"type":"string","description":"Preset compendium (default: Mass Edit's working pack)."}},
            "required":["document_name","ids","name"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

// ------------------------------------------------------------- logique pure

/// Critères de sélection d'un preset : `uuid` seul, `query` seul, ou
/// name/tags/types — Mass Edit lève une exception muette sur query + autres.
fn check_selector(args: &Value, required: bool) -> Result<()> {
    let has = |k: &str| match args.get(k) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(_) => true,
    };
    let fields = has("name") || has("tags") || has("types");
    if has("query") && fields {
        bail!(
            "query cannot be combined with name/tags/types (Mass Edit rejects it) — put everything in the query, e.g. \"lamp #tavern @AmbientLight\""
        );
    }
    if required && !has("uuid") && !has("query") && !fields {
        bail!("Provide uuid, query, or name/tags/types to select a preset");
    }
    Ok(())
}

/// Arguments de pose transmis au compagnon (x, y validés, pivot normalisé).
fn spawn_args(args: &Value) -> Result<Value> {
    check_selector(args, true)?;
    let num = |k: &str| {
        args.get(k)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .ok_or_else(|| anyhow!("'{k}' (canvas pixels) is required"))
    };
    let mut a = json!({"x": num("x")?, "y": num("y")?});
    if let Some(p) = str_arg(args, "pivot") {
        let p = p.trim().to_uppercase();
        if !PIVOTS.contains(&p.as_str()) {
            bail!("pivot must be one of {}", PIVOTS.join(", "));
        }
        a["pivot"] = json!(p);
    }
    carry(
        args,
        &[
            "uuid",
            "query",
            "name",
            "tags",
            "types",
            "random",
            "scene",
            "hidden",
            "snap_to_grid",
            "scale_to_grid",
        ],
        &mut a,
    );
    Ok(a)
}

/// Presets d'un pack d'après son index MassEditMetaData et l'index du pack
/// (les noms ne sont que dans ce dernier). Filtres facultatifs.
fn presets_of_pack(
    pack: &str,
    meta: &Value,
    names: &Map<String, Value>,
    name: Option<&str>,
    tag: Option<&str>,
    doc_type: Option<&str>,
) -> Vec<Value> {
    let Some(index) = meta
        .pointer(&format!("/flags/{MODULE}/index"))
        .and_then(Value::as_object)
    else {
        return vec![];
    };
    let name = name.map(|s| s.to_lowercase());
    let tag = tag.map(|s| s.trim().to_lowercase());
    let mut out: Vec<Value> = index
        .iter()
        .filter(|(id, _)| id.as_str() != META_INDEX_ID)
        .filter_map(|(id, e)| {
            // Une entrée dont le document a disparu est ignorée, comme le module.
            let pname = names.get(id)?.as_str().unwrap_or("");
            let tags: Vec<String> = e
                .get("tags")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            let dn = e.get("documentName").and_then(Value::as_str).unwrap_or("");
            if let Some(n) = &name
                && !pname.to_lowercase().contains(n.as_str())
            {
                return None;
            }
            if let Some(t) = &tag
                && !tags.iter().any(|x| x.to_lowercase() == *t)
            {
                return None;
            }
            if let Some(t) = doc_type
                && !dn.eq_ignore_ascii_case(t)
            {
                return None;
            }
            Some(json!({
                "uuid": format!("Compendium.{pack}.JournalEntry.{id}"),
                "name": pname, "documentName": dn, "tags": tags,
                "img": e.get("img").cloned().unwrap_or(Value::Null), "pack": pack,
            }))
        })
        .collect();
    out.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    out
}

// ------------------------------------------------------------------- outils

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "me_list_presets" => {
            let packs: Vec<String> = match str_arg(args, "pack") {
                Some(p) => vec![p],
                None => {
                    let world = foundry.request_world().await?;
                    world
                        .get("packs")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter(|p| {
                                    p.get("type")
                                        .or(p.get("documentName"))
                                        .and_then(Value::as_str)
                                        == Some("JournalEntry")
                                })
                                .filter_map(|p| {
                                    p.get("id")
                                        .or(p.get("collection"))
                                        .and_then(Value::as_str)
                                        .map(String::from)
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }
            };
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(200)
                .max(1) as usize;
            let (fname, ftag, ftype) = (
                str_arg(args, "name"),
                str_arg(args, "tag"),
                str_arg(args, "type"),
            );
            let mut found = Vec::new();
            let mut preset_packs = Vec::new();
            for pack in &packs {
                let mut q = Map::new();
                q.insert("_id".into(), json!(META_INDEX_ID));
                let meta = foundry
                    .get_collection("JournalEntry", q, false, Some(pack))
                    .await?;
                let Some(meta) = meta.into_iter().next() else {
                    continue;
                };
                preset_packs.push(pack.clone());
                let names: Map<String, Value> = foundry
                    .get_collection("JournalEntry", Map::new(), true, Some(pack))
                    .await?
                    .into_iter()
                    .filter_map(|d| {
                        Some((d.get("_id")?.as_str()?.to_string(), d.get("name")?.clone()))
                    })
                    .collect();
                found.extend(presets_of_pack(
                    pack,
                    &meta,
                    &names,
                    fname.as_deref(),
                    ftag.as_deref(),
                    ftype.as_deref(),
                ));
            }
            if str_arg(args, "pack").is_some() && preset_packs.is_empty() {
                bail!(
                    "{} is not a Mass Edit preset pack (no {META_INDEX_ID} document) or does not exist",
                    packs[0]
                );
            }
            let total = found.len();
            found.truncate(limit);
            Ok(text_response(&json!({
                "packs": preset_packs, "total": total, "returned": found.len(),
                "truncated": total > found.len(), "presets": found,
            })))
        }
        "client_me_find_presets" => {
            check_selector(args, true)?;
            let mut a = json!({});
            carry(
                args,
                &[
                    "uuid",
                    "query",
                    "name",
                    "tags",
                    "types",
                    "include_files",
                    "limit",
                ],
                &mut a,
            );
            let r = call_companion(state, cmd_for(name), a, None, 30).await?;
            Ok(text_response(&r))
        }
        "client_me_spawn_preset" => {
            let a = spawn_args(args)?;
            // Le mode « confirm » peut attendre le MJ ~25 s.
            let r = call_companion(state, cmd_for(name), a, None, 45).await?;
            Ok(text_response(&r))
        }
        "client_me_create_preset" => {
            let ids = args.get("ids").and_then(Value::as_array);
            if ids.is_none_or(|a| a.is_empty()) {
                bail!("'ids' (array of placeable ids) is required");
            }
            if str_arg(args, "name").is_none_or(|n| n.trim().is_empty()) {
                bail!("'name' is required");
            }
            let mut a = json!({});
            carry(
                args,
                &[
                    "document_name",
                    "ids",
                    "attached",
                    "name",
                    "tags",
                    "img",
                    "pack",
                ],
                &mut a,
            );
            let r = call_companion(state, cmd_for(name), a, None, 45).await?;
            Ok(text_response(&r))
        }
        _ => bail!("Unknown tool: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selecteur_query_exclusif() {
        assert!(check_selector(&json!({"query": "lamp #x"}), true).is_ok());
        assert!(check_selector(&json!({"name": "lamp", "tags": ["x"]}), true).is_ok());
        assert!(check_selector(&json!({"uuid": "Compendium.a.b.JournalEntry.c"}), true).is_ok());
        assert!(check_selector(&json!({"query": "lamp", "tags": "x"}), true).is_err());
        assert!(check_selector(&json!({"query": "lamp", "types": ["Tile"]}), true).is_err());
        // Champs vides = absents.
        assert!(check_selector(&json!({"query": "lamp", "name": "", "tags": []}), true).is_ok());
        assert!(check_selector(&json!({}), true).is_err());
        assert!(check_selector(&json!({}), false).is_ok());
    }

    #[test]
    fn arguments_de_pose() {
        let a = spawn_args(
            &json!({"uuid": "u", "x": 100, "y": 250.5, "pivot": "center", "hidden": true}),
        )
        .unwrap();
        assert_eq!(a["x"], json!(100.0));
        assert_eq!(a["y"], json!(250.5));
        assert_eq!(a["pivot"], "CENTER");
        assert_eq!(a["hidden"], true);
        assert!(spawn_args(&json!({"uuid": "u", "x": 1})).is_err());
        assert!(spawn_args(&json!({"uuid": "u", "x": 1, "y": "2"})).is_err());
        assert!(spawn_args(&json!({"uuid": "u", "x": 1, "y": 2, "pivot": "MIDDLE"})).is_err());
        assert!(spawn_args(&json!({"x": 1, "y": 2})).is_err());
    }

    #[test]
    fn index_des_presets() {
        let meta = json!({"_id": META_INDEX_ID, "flags": {"multi-token-edit": {"index": {
            "a": {"documentName": "AmbientLight", "tags": ["Taverne", "feu"], "img": "icons/svg/light.svg"},
            "b": {"documentName": "Tile", "tags": [], "img": "x.webp"},
            "gone": {"documentName": "Tile", "tags": []},
            META_INDEX_ID: {}
        }}}});
        let mut names = Map::new();
        names.insert("a".into(), json!("Lampe de taverne"));
        names.insert("b".into(), json!("Caisse"));
        names.insert(META_INDEX_ID.into(), json!("!!! METADATA"));
        let all = presets_of_pack("world.p", &meta, &names, None, None, None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["name"], "Caisse");
        assert_eq!(all[1]["uuid"], "Compendium.world.p.JournalEntry.a");
        let t = presets_of_pack("world.p", &meta, &names, None, Some("taverne"), None);
        assert_eq!(t.len(), 1);
        let n = presets_of_pack(
            "world.p",
            &meta,
            &names,
            Some("LAMPE"),
            None,
            Some("ambientlight"),
        );
        assert_eq!(n.len(), 1);
        assert!(presets_of_pack("world.p", &meta, &names, None, None, Some("Wall")).is_empty());
        assert!(presets_of_pack("world.p", &json!({}), &names, None, None, None).is_empty());
    }
}
