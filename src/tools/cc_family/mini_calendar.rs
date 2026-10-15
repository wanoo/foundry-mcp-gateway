//! Mini Calendar (mc_*) — temps (core.time), notes (journaux) + compagnon.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::mcp::McpState;
use crate::tools::companion::{Delivery, call_companion, cmd_for};
use crate::tools::{ToolDef, carry, str_arg, text_response};

/// Journaux où Mini Calendar range ses notes (relevés sur un monde réel).
const MC_NOTE_JOURNALS: [(&str, &str); 2] = [
    ("player", "Player Notes - Mini Calendar"),
    ("events", "Calendar Events - Mini Calendar"),
];

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

/// Outils `client_*` de ce module → commande compagnon (fusionnés dans
/// `companion::COMPANION_COMMANDS`).
pub const COMMANDS: &[(&str, &str, Delivery)] = &[
    ("client_mc_set_time", "mc_set_time", Delivery::Unique),
    ("client_mc_open", "mc_open", Delivery::Unique),
];

/// Outils sans effet de bord (annotations + mode FOUNDRY_READONLY).
pub const READ_ONLY: &[&str] = &["mc_get_time", "mc_list_notes"];

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    vec![
        // --- Mini Calendar ---
        ("mc_get_time",
         "Mini Calendar / core: read the world time (core.time, in seconds).",
         json!({"type":"object","properties":{}})),
        ("mc_set_time",
         "Mini Calendar / core: set the world time. Give world_time (absolute seconds) or advance_seconds (delta). The calendar updates on the world-time change.",
         json!({"type":"object","properties":{
            "world_time":{"type":"number"},"advance_seconds":{"type":"number"}}})),
        ("mc_list_notes",
         "Mini Calendar: read the calendar note journals (which: player | events | both). Notes are stored as journal pages.",
         json!({"type":"object","properties":{"which":{"type":"string","enum":["player","events","both"]}}})),
        ("client_mc_set_time",
         "Mini Calendar (companion): set the time via the game clock, including \"dawn\"/\"dusk\" of the current/next day (uses the module's setTime macro).",
         json!({"type":"object","properties":{
            "world_time":{"type":"number"},"advance_seconds":{"type":"number"},"mode":{"type":"string","enum":["dawn","dusk"]}}})),
        ("client_mc_open",
         "Mini Calendar (companion): open/toggle the calendar window on the GM client.",
         json!({"type":"object","properties":{}})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    #[allow(unused_variables)]
    let foundry = &state.foundry;
    match name {
        // ---------------------------------------------------- Mini Calendar
        "mc_get_time" => {
            let w = json!({"key": "core.time"}).as_object().cloned().unwrap();
            let docs = foundry
                .get_documents("settings", Some(&w), None, 0, Some(1))
                .await?;
            let raw = docs
                .first()
                .and_then(|d| d.get("value"))
                .and_then(Value::as_str)
                .unwrap_or("0");
            let seconds: i64 = raw.trim().parse().unwrap_or(0);
            Ok(text_response(&json!({"worldTime": seconds})))
        }
        "mc_set_time" => {
            let w = json!({"key": "core.time"}).as_object().cloned().unwrap();
            let docs = foundry
                .get_documents("settings", Some(&w), None, 0, Some(1))
                .await?;
            let doc = docs
                .first()
                .ok_or_else(|| anyhow!("core.time setting not found"))?;
            let current: i64 = doc
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("0")
                .trim()
                .parse()
                .unwrap_or(0);
            let target = match (
                args.get("world_time").and_then(Value::as_i64),
                args.get("advance_seconds").and_then(Value::as_i64),
            ) {
                (Some(t), _) => t,
                (None, Some(d)) => current + d,
                _ => bail!("Provide world_time or advance_seconds"),
            };
            let result = foundry
                .modify_document(
                    "Setting",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{"_id": doc["_id"], "value": target.to_string()}],
                    }),
                )
                .await?;
            Ok(text_response(
                &json!({"worldTime": target, "before": current, "result": result}),
            ))
        }
        "mc_list_notes" => {
            let which = str_arg(args, "which").unwrap_or_else(|| "both".into());
            let mut out = serde_json::Map::new();
            for (key, jname) in MC_NOTE_JOURNALS {
                if which != "both" && which != key {
                    continue;
                }
                let doc = foundry
                    .get_document("journal", None, Some(jname), None)
                    .await?;
                let pages = doc
                    .as_ref()
                    .and_then(|d| d.get("pages"))
                    .and_then(Value::as_array)
                    .map(|ps| {
                        ps.iter()
                            .map(|p| {
                                json!({
                                    "name": p.get("name"),
                                    "text": p.pointer("/text/content"),
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                out.insert(
                    key.to_string(),
                    json!({"journal": jname, "found": doc.is_some(), "pages": pages}),
                );
            }
            Ok(text_response(&Value::Object(out)))
        }
        "client_mc_set_time" => {
            let mut a = json!({});
            carry(args, &["world_time", "advance_seconds"], &mut a);
            if let Some(m) = str_arg(args, "mode") {
                a["mode"] = json!(m);
            }
            let r = call_companion(state, cmd_for(name), a, None, 10).await?;
            Ok(text_response(&r))
        }
        "client_mc_open" => {
            let r = call_companion(state, cmd_for(name), json!({}), None, 10).await?;
            Ok(text_response(&r))
        }

        other => bail!("Unknown tool: {other}"),
    }
}
