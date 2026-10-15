//! Prompts MCP : `prompts/list` et `prompts/get`. Voir `resources.rs` pour la
//! raison du découpage.

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::McpState;

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
