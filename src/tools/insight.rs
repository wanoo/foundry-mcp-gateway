//! Perception (compagnon) — ce que le protocole socket ne peut PAS donner, parce
//! que ça n'existe que dans le navigateur :
//!   · client_get_derived : les valeurs PRÉPARÉES d'une fiche (prepareData +
//!     effets actifs), là où le serveur ne voit que le document source.
//!   · client_enrich : le HTML enrichi (@UUID résolus, jets inline, secrets).
//!   · client_search : recherche sur l'index client, toutes collections.
//!   · client_capture : une image de la vue courante (contenu image MCP natif).
//!   · client_scene_report : l'état jouable de la scène (visibilité réelle).

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::companion::call_companion;
use super::{image_response, str_arg, text_response};
use crate::mcp::McpState;

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("client_get_derived",
         "THE reliable way to read a sheet: returns the PREPARED values (after the system's prepareData and active effects) for any document uuid — not the source document, whose stats are often zero/incomplete. Use this rather than get_actor whenever you need what the player actually sees. Optionally includes owned items and active effects.",
         json!({"type":"object","properties":{
            "uuid":{"type":"string","description":"e.g. Actor.abc123, Actor.abc.Item.def, JournalEntry.xyz"},
            "items":{"type":"boolean","description":"include owned items with their derived system data"},
            "effects":{"type":"boolean","description":"include active effects (default true)"}},
            "required":["uuid"]})),
        ("client_enrich",
         "Return the ENRICHED HTML of a document (or raw html): @UUID links resolved to names, inline rolls evaluated, secrets revealed. The server only ever sees the raw source text.",
         json!({"type":"object","properties":{
            "uuid":{"type":"string"},"html":{"type":"string"},
            "secrets":{"type":"boolean","description":"reveal GM secret blocks (default true)"}}})),
        ("client_search",
         "Search every world collection by name through the client index (Actors, Items, Journals, Scenes, Tables, Macros, Cards, Playlists). Returns uuid + folder for each hit.",
         json!({"type":"object","properties":{
            "query":{"type":"string"},
            "types":{"type":"array","items":{"type":"string"},
                     "description":"restrict to document types, e.g. [\"Actor\",\"JournalEntry\"]"},
            "limit":{"type":"number"}},
            "required":["query"]})),
        ("client_capture",
         "Screenshot the GM's current canvas view and return it as an image — literally see the table: token positions, lighting, fog, drawings. Pan/zoom first with client_pan_camera to frame it.",
         json!({"type":"object","properties":{
            "max_width":{"type":"number","description":"downscale before encoding (default 900)"},
            "quality":{"type":"number","description":"webp quality 0-1 (default 0.6)"}}})),
        ("client_babele",
         "Babele: the TRANSLATED view of compendia as the players see them (the server only ever reads the source language). 'query': REVERSE search — find documents by their displayed (translated) OR source name across all translated packs; use this when the user names things in their own language. No args: list translated packs. pack alone: translated index. pack + id/name/ids: full translated document(s).",
         json!({"type":"object","properties":{
            "query":{"type":"string","description":"search displayed or source names across translated packs"},
            "pack":{"type":"string","description":"compendium collection, e.g. starwarsffg.talents (restricts query too)"},
            "id":{"type":"string"},"name":{"type":"string"},
            "ids":{"type":"array","items":{"type":"string"},"description":"batch: several documents at once"},
            "limit":{"type":"number","description":"index/search entries (default 100)"}}})),
        ("client_scene_report",
         "Playable state of the active scene as the GM's client sees it: tokens with grid coordinates, disposition, and REAL visibility (vision + fog), doors and their open/closed state, lights, measured templates, current selection and targets.",
         json!({"type":"object","properties":{
            "include_walls":{"type":"boolean","description":"also dump every wall segment (verbose)"}}})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    match name {
        "client_get_derived" => {
            let uuid = str_arg(args, "uuid").ok_or_else(|| anyhow!("'uuid' is required"))?;
            let mut a = json!({ "uuid": uuid });
            for k in ["items", "effects"] {
                if let Some(v) = args.get(k) {
                    a[k] = v.clone();
                }
            }
            Ok(text_response(
                &call_companion(state, "get_derived", a, None, 15).await?,
            ))
        }
        "client_enrich" => {
            let mut a = json!({});
            for k in ["uuid", "html", "secrets"] {
                if let Some(v) = args.get(k) {
                    a[k] = v.clone();
                }
            }
            if a.get("uuid").is_none() && a.get("html").is_none() {
                bail!("give 'uuid' or 'html'");
            }
            Ok(text_response(
                &call_companion(state, "enrich", a, None, 15).await?,
            ))
        }
        "client_search" => {
            let query = str_arg(args, "query").ok_or_else(|| anyhow!("'query' is required"))?;
            let mut a = json!({ "query": query });
            for k in ["types", "limit"] {
                if let Some(v) = args.get(k) {
                    a[k] = v.clone();
                }
            }
            Ok(text_response(
                &call_companion(state, "search", a, None, 15).await?,
            ))
        }
        "client_capture" => {
            let mut a = json!({});
            for k in ["max_width", "quality"] {
                if let Some(v) = args.get(k) {
                    a[k] = v.clone();
                }
            }
            let r = call_companion(state, "capture", a, None, 30).await?;
            let b64 = r
                .get("base64")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("capture sans image"))?;
            let mime = r
                .get("mimeType")
                .and_then(Value::as_str)
                .unwrap_or("image/webp");
            let caption = json!({
                "scene": r.get("scene"), "width": r.get("width"), "height": r.get("height"),
            });
            Ok(image_response(b64, mime, &caption))
        }
        "client_babele" => {
            let mut a = json!({});
            for k in ["query", "pack", "id", "name", "ids", "limit"] {
                if let Some(v) = args.get(k) {
                    a[k] = v.clone();
                }
            }
            Ok(text_response(
                &call_companion(state, "babele", a, None, 20).await?,
            ))
        }
        "client_scene_report" => {
            let mut a = json!({});
            if let Some(v) = args.get("include_walls") {
                a["include_walls"] = v.clone();
            }
            Ok(text_response(
                &call_companion(state, "scene_report", a, None, 20).await?,
            ))
        }
        other => bail!("Unknown tool: {other}"),
    }
}
