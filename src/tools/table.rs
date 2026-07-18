//! La table (compagnon) — interaction et ambiance qui n'existent que dans le
//! navigateur :
//!   · client_ask : poser une question à un joueur et RÉCUPÉRER sa réponse
//!     (seul outil dont la réponse vient du client ciblé, pas du responder).
//!   · client_select / client_target : sélection et ciblage réels.
//!   · client_fog : réinitialiser le brouillard exploré.
//!   · FXMaster (météo), Token Magic FX (filtres de token), catalogue Sequencer.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::companion::call_companion;
use super::{str_arg, text_response};
use crate::mcp::McpState;

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("client_ask",
         "Ask a player a question in a real dialog on THEIR screen and wait for their answer. targets: 'gm' / 'players' / [userId] (get ids from client_get_state). Returns the chosen option, or answered=false if they dismissed it or the timeout expired.",
         json!({"type":"object","properties":{
            "question":{"type":"string"},
            "options":{"type":"array","items":{"type":"string"},"description":"button labels (default Oui/Non)"},
            "title":{"type":"string"},
            "targets":{},
            "timeout_seconds":{"type":"number","description":"default 120"}},
            "required":["question"]})),
        ("client_roll_formula",
         "Roll ANY dice formula with Foundry's real Roll engine, on any game system: posts the native chat card and rolls 3D dice if Dice So Nice is installed. Returns the total plus every individual die. Use this for generic rolls (2d6+3, 1d100, @str+2 with an actor's roll data); system tools like roll_actor_skill remain better for system-specific mechanics.",
         json!({"type":"object","properties":{
            "formula":{"type":"string","description":"e.g. \"2d6+3\", \"1d20+@abilities.dex.mod\""},
            "flavor":{"type":"string","description":"chat card label"},
            "actor":{"type":"string","description":"actor _id: speaker + roll data (@attributes)"},
            "whisper_gm":{"type":"boolean","description":"whisper the result to GMs only"},
            "roll_data":{"type":"object","additionalProperties":true,"description":"extra @variables"}},
            "required":["formula"]})),
        ("client_select",
         "Select tokens on the GM's canvas (real client selection, not a document change) — so the GM sees what you are talking about.",
         json!({"type":"object","properties":{
            "tokens":{"type":"array","items":{"type":"string"},"description":"token _ids on the active scene"},
            "release_others":{"type":"boolean","description":"clear the current selection first (default true)"}},
            "required":["tokens"]})),
        ("client_target",
         "Set the GM's targets (the crosshair markers other players see) on the given tokens.",
         json!({"type":"object","properties":{
            "tokens":{"type":"array","items":{"type":"string"}},
            "release_others":{"type":"boolean"}},
            "required":["tokens"]})),
        ("client_fog",
         "Reset the explored fog of war on the active scene (re-hides everything the party has revealed).",
         json!({"type":"object","properties":{
            "action":{"type":"string","description":"reset (default)"}}})),
        ("client_weather",
         "FXMaster: set the scene's weather particle effects (rain, fog, snow, embers, clouds, bats…). Pass effects as names or {type, options}; empty list or clear=true stops the weather. List what is available with client_weather_types.",
         json!({"type":"object","properties":{
            "effects":{"type":"array","items":{},"description":"e.g. [\"rain\"] or [{\"type\":\"rain\",\"options\":{\"density\":80}}]"},
            "clear":{"type":"boolean"}}})),
        ("client_weather_types",
         "FXMaster: the particle effect types available in this world.",
         json!({"type":"object","properties":{}})),
        ("client_token_fx",
         "Token Magic FX: apply (or remove) a filter preset on tokens — glow, blur, fire, shadow… See client_token_fx_presets.",
         json!({"type":"object","properties":{
            "tokens":{"type":"array","items":{"type":"string"}},
            "preset":{"type":"string"},
            "remove":{"type":"boolean","description":"remove instead of apply (omit preset to strip all)"}},
            "required":["tokens"]})),
        ("client_token_fx_presets",
         "Token Magic FX: the filter presets available in this world.",
         json!({"type":"object","properties":{}})),
        ("client_effect_catalog",
         "Sequencer: search the installed effect database (JB2A & co) to find a valid file path BEFORE calling client_play_effect or client_seq_between. Give a fuzzy 'query' or a db path in 'under'; with neither, returns the installed effect modules.",
         json!({"type":"object","properties":{
            "query":{"type":"string","description":"fuzzy search, e.g. \"lightning\""},
            "under":{"type":"string","description":"db path, e.g. \"jb2a.explosion\""},
            "limit":{"type":"number"}}})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

/// Copie les clés présentes d'un Value vers un autre (arguments optionnels).
fn carry(args: &Value, keys: &[&str], out: &mut Value) {
    for k in keys {
        if let Some(v) = args.get(*k) {
            out[*k] = v.clone();
        }
    }
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let targets = args.get("targets").cloned();
    let result = match name {
        "client_ask" => {
            let question =
                str_arg(args, "question").ok_or_else(|| anyhow!("'question' is required"))?;
            let mut a = json!({ "question": question });
            carry(args, &["options", "title", "timeout_seconds"], &mut a);
            // On attend un peu plus longtemps que le dialogue lui-même, sinon
            // l'appel expire avant la réponse du joueur.
            let wait = args
                .get("timeout_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(120)
                + 15;
            call_companion(state, "ask", a, targets, wait).await?
        }
        "client_roll_formula" => {
            let formula =
                str_arg(args, "formula").ok_or_else(|| anyhow!("'formula' is required"))?;
            let mut a = json!({ "formula": formula });
            carry(args, &["flavor", "whisper_gm", "roll_data"], &mut a);
            if let Some(actor) = str_arg(args, "actor") {
                a["actorId"] = json!(actor);
            }
            call_companion(state, "roll_formula", a, None, 30).await?
        }
        "client_select" | "client_target" => {
            let tokens = args
                .get("tokens")
                .cloned()
                .ok_or_else(|| anyhow!("'tokens' is required"))?;
            let mut a = json!({ "tokens": tokens });
            carry(args, &["release_others"], &mut a);
            let cmd = if name == "client_select" {
                "select"
            } else {
                "target"
            };
            call_companion(state, cmd, a, None, 10).await?
        }
        "client_fog" => {
            let mut a = json!({});
            carry(args, &["action"], &mut a);
            call_companion(state, "fog", a, None, 20).await?
        }
        "client_weather" => {
            let mut a = json!({});
            carry(args, &["effects", "clear"], &mut a);
            call_companion(state, "weather", a, None, 15).await?
        }
        "client_weather_types" => {
            call_companion(state, "weather_types", json!({}), None, 10).await?
        }
        "client_token_fx" => {
            let tokens = args
                .get("tokens")
                .cloned()
                .ok_or_else(|| anyhow!("'tokens' is required"))?;
            let mut a = json!({ "tokens": tokens });
            carry(args, &["preset", "remove"], &mut a);
            call_companion(state, "token_fx", a, None, 15).await?
        }
        "client_token_fx_presets" => {
            call_companion(state, "token_fx_presets", json!({}), None, 10).await?
        }
        "client_effect_catalog" => {
            let mut a = json!({});
            carry(args, &["query", "under", "limit"], &mut a);
            call_companion(state, "effect_catalog", a, None, 15).await?
        }
        other => bail!("Unknown tool: {other}"),
    };
    Ok(text_response(&result))
}
