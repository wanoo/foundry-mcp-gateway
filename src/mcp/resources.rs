//! Ressources MCP : expose le monde en lecture via `resources/list` et
//! `resources/read`. Vit ici et non dans `tools/` — ce sont deux surfaces
//! distinctes de la spec MCP, et leur cohabitation dans le registre d'outils
//! était un accident d'historique.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::McpState;
use crate::foundry::documents::get_path;

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
