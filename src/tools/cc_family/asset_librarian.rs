//! Asset Librarian (al_*) — tags flags["asset-librarian"] + navigateur (compagnon).

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::mcp::McpState;
use crate::tools::companion::{Delivery, call_companion, cmd_for};
use crate::tools::{ToolDef, str_arg, text_response};

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

/// Outils `client_*` de ce module → commande compagnon (fusionnés dans
/// `companion::COMPANION_COMMANDS`).
pub const COMMANDS: &[(&str, &str, Delivery)] = &[("client_al_open", "al_open", Delivery::Unique)];

/// Outils sans effet de bord (annotations + mode FOUNDRY_READONLY).
pub const READ_ONLY: &[&str] = &["al_find"];

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    vec![
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
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    #[allow(unused_variables)]
    let foundry = &state.foundry;
    match name {
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
            let r = call_companion(state, cmd_for(name), a, None, 10).await?;
            Ok(text_response(&r))
        }

        other => bail!("Unknown tool: {other}"),
    }
}
