//! Module daggerheart : dés de Dualité du SRD + 3 outils (préfixe dh_).
//! Chemins vérifiés sur le système Foundryborne/daggerheart :
//! system.traits.<trait>.value · system.resources.{hitPoints,stress,hope}.{value,max}.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::foundry::documents::get_path;
use crate::mcp::McpState;
use crate::tools::{post_chat, str_arg, text_response};

pub const TRAITS: [&str; 6] = ["agility", "strength", "finesse", "instinct", "presence", "knowledge"];
const STAT_PATHS: [(&str, &str); 3] = [
    ("hit_points", "system.resources.hitPoints.value"),
    ("stress", "system.resources.stress.value"),
    ("hope", "system.resources.hope.value"),
];

pub struct DualityResult {
    pub hope: i64,
    pub fear: i64,
    pub advantage_die: Option<i64>,
    pub total: i64,
    pub is_critical: bool,
    pub with_hope: bool,
    pub success: Option<bool>,
}

pub fn roll_duality<R: FnMut() -> f64>(
    modifier: i64, difficulty: Option<i64>, advantage: bool, disadvantage: bool, mut rng: R,
) -> DualityResult {
    let d12 = |rng: &mut R| 1 + (rng() * 12.0) as i64 % 12;
    let d6 = |rng: &mut R| 1 + (rng() * 6.0) as i64 % 6;
    let hope = d12(&mut rng);
    let fear = d12(&mut rng);
    let adv = advantage && !disadvantage;
    let dis = disadvantage && !advantage;
    let advantage_die = if adv { Some(d6(&mut rng)) } else if dis { Some(-d6(&mut rng)) } else { None };
    let total = hope + fear + modifier + advantage_die.unwrap_or(0);
    let is_critical = hope == fear;
    DualityResult {
        hope, fear, advantage_die, total, is_critical,
        with_hope: hope > fear,
        success: difficulty.map(|d| is_critical || total >= d),
    }
}

pub fn format_duality(r: &DualityResult) -> String {
    let mut parts = vec![format!("Espoir {} + Peur {}", r.hope, r.fear)];
    if let Some(d) = r.advantage_die {
        parts.push(if d >= 0 { format!("+ d6 avantage {d}") } else { format!("− d6 désavantage {}", -d) });
    }
    let outcome = if r.is_critical {
        "⭐ RÉUSSITE CRITIQUE (gagne 1 Espoir, efface 1 Stress)".to_string()
    } else {
        match (r.success, r.with_hope) {
            (Some(true), true) => "✅ Réussite avec Espoir".into(),
            (Some(true), false) => "✅ Réussite avec Peur".into(),
            (Some(false), true) => "❌ Échec avec Espoir".into(),
            (Some(false), false) => "❌ Échec avec Peur".into(),
            (None, true) => "avec Espoir (+1 Espoir)".into(),
            (None, false) => "avec Peur (+1 Peur au MJ)".into(),
        }
    };
    format!("{} = {} · {outcome}", parts.join(" "), r.total)
}

fn result_json(r: &DualityResult) -> Value {
    json!({
        "hope": r.hope, "fear": r.fear, "advantageDie": r.advantage_die,
        "total": r.total, "isCritical": r.is_critical,
        "withHope": r.with_hope, "withFear": !r.with_hope && r.fear > r.hope,
        "success": r.success,
    })
}

pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("dh_roll_duality",
         "Daggerheart Duality Dice (2d12 Hope+Fear): doubles = critical success, advantage ±d6. Posts to chat.",
         json!({"type":"object","properties":{
            "description":{"type":"string"},"modifier":{"type":"number"},
            "difficulty":{"type":"number"},"advantage":{"type":"boolean"},
            "disadvantage":{"type":"boolean"},"post":{"type":"boolean"},
            "whisper_users":{"type":"array","items":{"type":"string"}}},
            "required":["description"]})),
        ("dh_roll_actor_trait",
         "Duality roll FOR a Daggerheart actor using one of its traits (agility/strength/finesse/instinct/presence/knowledge).",
         json!({"type":"object","properties":{
            "actor":{"type":"string"},"trait":{"type":"string","enum":TRAITS},
            "difficulty":{"type":"number"},"advantage":{"type":"boolean"},
            "disadvantage":{"type":"boolean"},"extra_modifier":{"type":"number"},
            "post":{"type":"boolean"},"whisper_users":{"type":"array","items":{"type":"string"}}},
            "required":["actor","trait"]})),
        ("dh_adjust_stats",
         "Adjust Daggerheart resources: hit_points (marked HP), stress, hope. DELTAS by default; clamped to [0, max].",
         json!({"type":"object","properties":{
            "actor":{"type":"string"},"set":{"type":"boolean"},
            "hit_points":{"type":"number"},"stress":{"type":"number"},"hope":{"type":"number"}},
            "required":["actor"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "dh_roll_duality" | "dh_roll_actor_trait" => {
            let (modifier, title, actor_ref) = if name == "dh_roll_duality" {
                let description = str_arg(args, "description")
                    .ok_or_else(|| anyhow!("'description' is required"))?;
                (args.get("modifier").and_then(Value::as_i64).unwrap_or(0), description, None)
            } else {
                let actor_arg = str_arg(args, "actor").ok_or_else(|| anyhow!("'actor' is required"))?;
                let trait_name = str_arg(args, "trait").map(|t| t.to_lowercase())
                    .ok_or_else(|| anyhow!("'trait' is required"))?;
                if !TRAITS.contains(&trait_name.as_str()) {
                    bail!("Unknown trait '{trait_name}'. Valid: {}", TRAITS.join(", "));
                }
                let actor = foundry.find_document("actors", &actor_arg, None).await?
                    .ok_or_else(|| anyhow!("Actor not found: {actor_arg}"))?;
                let trait_value = get_path(&actor, &format!("system.traits.{trait_name}.value"))
                    .and_then(Value::as_i64).unwrap_or(0);
                let modifier = trait_value + args.get("extra_modifier").and_then(Value::as_i64).unwrap_or(0);
                let title = format!(
                    "{} — {}{} ({trait_value:+})",
                    actor["name"].as_str().unwrap_or("?"),
                    trait_name[..1].to_uppercase(), &trait_name[1..]
                );
                (modifier, title, Some(actor))
            };
            let difficulty = args.get("difficulty").and_then(Value::as_i64);
            let roll = {
                let mut rng = rand::rng();
                roll_duality(
                    modifier, difficulty,
                    args.get("advantage").and_then(Value::as_bool).unwrap_or(false),
                    args.get("disadvantage").and_then(Value::as_bool).unwrap_or(false),
                    || rand::Rng::random::<f64>(&mut rng),
                )
            };
            let summary = format_duality(&roll);
            let mut posted = false;
            if args.get("post").and_then(Value::as_bool).unwrap_or(true) {
                let content = format!(
                    "<h3>🎲 {title}{}</h3><p><strong>{summary}</strong></p>",
                    difficulty.map(|d| format!(" (Difficulté {d})")).unwrap_or_default()
                );
                post_chat(state, &content,
                    json!({"foundry-mcp": {"duality": result_json(&roll)}}),
                    args.get("whisper_users")).await?;
                posted = true;
            }
            let mut out = result_json(&roll);
            out["modifier"] = json!(modifier);
            out["summary"] = json!(summary);
            out["posted"] = json!(posted);
            if let Some(a) = actor_ref {
                out["actor"] = json!({"_id": a["_id"], "name": a["name"]});
            }
            Ok(text_response(&out))
        }
        "dh_adjust_stats" => {
            let actor_arg = str_arg(args, "actor").ok_or_else(|| anyhow!("'actor' is required"))?;
            let requested: Vec<&(&str, &str)> = STAT_PATHS.iter()
                .filter(|(k, _)| args.get(*k).is_some()).collect();
            if requested.is_empty() {
                bail!("provide at least one of: hit_points, stress, hope");
            }
            let actor = foundry.find_document("actors", &actor_arg, None).await?
                .ok_or_else(|| anyhow!("Actor not found: {actor_arg}"))?;
            let absolute = args.get("set").and_then(Value::as_bool).unwrap_or(false);
            let mut update = serde_json::Map::new();
            let mut changes = serde_json::Map::new();
            for (key, path) in requested {
                let before = get_path(&actor, path).and_then(Value::as_i64).unwrap_or(0);
                let max = get_path(&actor, &path.replace(".value", ".max")).and_then(Value::as_i64);
                let input = args.get(*key).and_then(Value::as_i64).unwrap_or(0);
                let mut after = (if absolute { input } else { before + input }).max(0);
                if let Some(m) = max.filter(|m| *m > 0) { after = after.min(m); }
                update.insert((*path).into(), json!(after));
                changes.insert((*key).into(), json!({"before": before, "after": after}));
            }
            let mut update_doc = Value::Object(update);
            update_doc["_id"] = actor["_id"].clone();
            let result = foundry.modify_document("Actor", "update", json!({
                "action": "update", "diff": false, "recursive": true, "render": true,
                "updates": [update_doc],
            })).await?;
            Ok(text_response(&json!({
                "actor": {"_id": actor["_id"], "name": actor["name"]},
                "mode": if absolute { "set" } else { "delta" },
                "changes": changes, "result": result,
            })))
        }
        other => bail!("Unknown tool: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(vals: Vec<f64>) -> impl FnMut() -> f64 {
        let mut i = 0;
        move || { let v = vals[i % vals.len()]; i += 1; v }
    }
    fn d12(n: i64) -> f64 { (n - 1) as f64 / 12.0 }

    #[test]
    fn espoir_peur_critique() {
        let r = roll_duality(2, Some(12), false, false, seq(vec![d12(8), d12(5)]));
        assert!(r.with_hope && r.success == Some(true) && r.total == 15);
        let crit = roll_duality(0, Some(20), false, false, seq(vec![d12(4), d12(4)]));
        assert!(crit.is_critical && crit.success == Some(true));
        assert!(format_duality(&crit).contains("CRITIQUE"));
    }

    #[test]
    fn avantage_desavantage() {
        let adv = roll_duality(0, None, true, false, seq(vec![d12(6), d12(3), 3.0 / 6.0]));
        assert_eq!((adv.advantage_die, adv.total), (Some(4), 13));
        let dis = roll_duality(0, None, false, true, seq(vec![d12(6), d12(3), 3.0 / 6.0]));
        assert_eq!((dis.advantage_die, dis.total), (Some(-4), 5));
        let both = roll_duality(0, None, true, true, seq(vec![d12(6), d12(3)]));
        assert!(both.advantage_die.is_none());
    }
}
