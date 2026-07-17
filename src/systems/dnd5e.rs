//! Module dnd5e : moteur d20 SRD + 2 outils (préfixe dnd5e_).
//! Chemins du système dnd5e (stables) : abilities.<ab>.{value,proficient},
//! skills.<sk>.{value,ability}, attributes.hp.{value,max,temp}, details.{level,xp,cr},
//! attributes.exhaustion, currency.{pp,gp,ep,sp,cp}.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::foundry::documents::get_path;
use crate::mcp::McpState;
use crate::tools::{post_chat, str_arg, text_response};

const ABILITIES: [(&str, &str); 6] = [
    ("str", "Strength"),
    ("dex", "Dexterity"),
    ("con", "Constitution"),
    ("int", "Intelligence"),
    ("wis", "Wisdom"),
    ("cha", "Charisma"),
];
const SKILLS: [(&str, &str, &str); 18] = [
    ("acr", "dex", "Acrobatics"),
    ("ani", "wis", "Animal Handling"),
    ("arc", "int", "Arcana"),
    ("ath", "str", "Athletics"),
    ("dec", "cha", "Deception"),
    ("his", "int", "History"),
    ("ins", "wis", "Insight"),
    ("itm", "cha", "Intimidation"),
    ("inv", "int", "Investigation"),
    ("med", "wis", "Medicine"),
    ("nat", "int", "Nature"),
    ("prc", "wis", "Perception"),
    ("prf", "cha", "Performance"),
    ("per", "cha", "Persuasion"),
    ("rel", "int", "Religion"),
    ("slt", "dex", "Sleight of Hand"),
    ("ste", "dex", "Stealth"),
    ("sur", "wis", "Survival"),
];
const STAT_PATHS: [(&str, &str); 9] = [
    ("hp", "system.attributes.hp.value"),
    ("temp_hp", "system.attributes.hp.temp"),
    ("xp", "system.details.xp.value"),
    ("exhaustion", "system.attributes.exhaustion"),
    ("pp", "system.currency.pp"),
    ("gp", "system.currency.gp"),
    ("ep", "system.currency.ep"),
    ("sp", "system.currency.sp"),
    ("cp", "system.currency.cp"),
];

pub fn ability_modifier(score: i64) -> i64 {
    (score - 10).div_euclid(2)
}
pub fn proficiency_bonus(level: i64) -> i64 {
    2 + (level.max(1) - 1) / 4
}

pub struct D20Result {
    pub rolls: Vec<i64>,
    pub kept: i64,
    pub total: i64,
    pub crit: bool,
    pub fumble: bool,
    pub success: Option<bool>,
}

pub fn roll_d20<R: FnMut() -> f64>(
    modifier: i64,
    advantage: bool,
    disadvantage: bool,
    dc: Option<i64>,
    mut rng: R,
) -> D20Result {
    let d20 = |rng: &mut R| 1 + (rng() * 20.0) as i64 % 20;
    let adv = advantage && !disadvantage;
    let dis = disadvantage && !advantage;
    let rolls = if adv || dis {
        vec![d20(&mut rng), d20(&mut rng)]
    } else {
        vec![d20(&mut rng)]
    };
    let kept = if adv {
        *rolls.iter().max().unwrap()
    } else if dis {
        *rolls.iter().min().unwrap()
    } else {
        rolls[0]
    };
    let total = kept + modifier;
    D20Result {
        crit: kept == 20,
        fumble: kept == 1,
        success: dc.map(|d| {
            if kept == 20 {
                true
            } else if kept == 1 {
                false
            } else {
                total >= d
            }
        }),
        rolls,
        kept,
        total,
    }
}

pub fn format_d20(r: &D20Result, modifier: i64) -> String {
    let dice = if r.rolls.len() == 2 {
        format!("[{}, {}] → {}", r.rolls[0], r.rolls[1], r.kept)
    } else {
        r.kept.to_string()
    };
    let outcome = match r.success {
        Some(true) => " · ✅ réussite",
        Some(false) => " · ❌ échec",
        None => "",
    };
    let special = if r.crit {
        " · ⭐ CRITIQUE"
    } else if r.fumble {
        " · 💀 ÉCHEC CRITIQUE"
    } else {
        ""
    };
    format!("d20 {dice} {modifier:+} = {}{outcome}{special}", r.total)
}

pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("dnd5e_roll_check",
         "d20 check FOR a dnd5e actor: ability (\"str\"), skill (\"athletics\"/\"ath\"), or save (\"dex_save\"). Modifier derived from the sheet; advantage/disadvantage cancel; nat 20/1 auto.",
         json!({"type":"object","properties":{
            "actor":{"type":"string"},"check":{"type":"string"},"dc":{"type":"number"},
            "advantage":{"type":"boolean"},"disadvantage":{"type":"boolean"},
            "post":{"type":"boolean"},"whisper_users":{"type":"array","items":{"type":"string"}}},
            "required":["actor","check"]})),
        ("dnd5e_adjust_stats",
         "Adjust dnd5e stats: hp (clamped to max), temp_hp, xp, exhaustion, currency pp/gp/ep/sp/cp. DELTAS by default; set:true = absolute.",
         json!({"type":"object","properties":{
            "actor":{"type":"string"},"set":{"type":"boolean"},
            "hp":{"type":"number"},"temp_hp":{"type":"number"},"xp":{"type":"number"},
            "exhaustion":{"type":"number"},"pp":{"type":"number"},"gp":{"type":"number"},
            "ep":{"type":"number"},"sp":{"type":"number"},"cp":{"type":"number"}},
            "required":["actor"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "dnd5e_roll_check" => {
            let actor_arg = str_arg(args, "actor").ok_or_else(|| anyhow!("'actor' is required"))?;
            let check = str_arg(args, "check")
                .ok_or_else(|| anyhow!("'check' is required"))?
                .to_lowercase()
                .trim()
                .to_string();
            let actor = foundry
                .find_document("actors", &actor_arg, None)
                .await?
                .ok_or_else(|| anyhow!("Actor not found: {actor_arg}"))?;
            let num = |p: &str| get_path(&actor, p).and_then(Value::as_i64);
            let level = num("system.details.level")
                .or_else(|| num("system.details.cr"))
                .unwrap_or(1);
            let prof = proficiency_bonus(level);
            let score = |ab: &str| num(&format!("system.abilities.{ab}.value")).unwrap_or(10);

            let (label, modifier) = if let Some(ab) = check
                .strip_suffix("_save")
                .or_else(|| check.strip_suffix(" save"))
                .filter(|a| ABILITIES.iter().any(|(k, _)| k == a))
            {
                let proficient = num(&format!("system.abilities.{ab}.proficient")).unwrap_or(0);
                let full = ABILITIES.iter().find(|(k, _)| *k == ab).unwrap().1;
                (
                    format!("Sauvegarde de {full}"),
                    ability_modifier(score(ab)) + proficient * prof,
                )
            } else if let Some((_, full)) = ABILITIES.iter().find(|(k, _)| *k == check) {
                (format!("Test de {full}"), ability_modifier(score(&check)))
            } else if let Some((key, default_ab, label)) = SKILLS
                .iter()
                .find(|(k, _, l)| *k == check || l.to_lowercase() == check)
            {
                let ability = get_path(&actor, &format!("system.skills.{key}.ability"))
                    .and_then(Value::as_str)
                    .unwrap_or(default_ab)
                    .to_string();
                // multiplicateur 0 / 0.5 / 1 / 2 (expertise)
                let mult = get_path(&actor, &format!("system.skills.{key}.value"))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                (
                    label.to_string(),
                    ability_modifier(score(&ability)) + (mult * prof as f64).floor() as i64,
                )
            } else {
                bail!(
                    "Unknown check '{check}'. Abilities: {} (+_save) · skills: {}",
                    ABILITIES
                        .iter()
                        .map(|(k, _)| *k)
                        .collect::<Vec<_>>()
                        .join("/"),
                    SKILLS
                        .iter()
                        .map(|(_, _, l)| *l)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            };

            let dc = args.get("dc").and_then(Value::as_i64);
            let result = {
                let mut rng = rand::rng();
                roll_d20(
                    modifier,
                    args.get("advantage")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    args.get("disadvantage")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    dc,
                    || rand::Rng::random::<f64>(&mut rng),
                )
            };
            let summary = format_d20(&result, modifier);
            let mut posted = false;
            if args.get("post").and_then(Value::as_bool).unwrap_or(true) {
                let content = format!(
                    "<h3>🎲 {} — {label}{}</h3><p><strong>{summary}</strong></p>",
                    actor["name"].as_str().unwrap_or("?"),
                    dc.map(|d| format!(" (DD {d})")).unwrap_or_default()
                );
                post_chat(
                    state,
                    &content,
                    json!({"foundry-mcp": {"d20": {"actor": actor["_id"], "check": check}}}),
                    args.get("whisper_users"),
                )
                .await?;
                posted = true;
            }
            Ok(text_response(&json!({
                "actor": {"_id": actor["_id"], "name": actor["name"]},
                "check": label, "modifier": modifier,
                "rolls": result.rolls, "kept": result.kept, "total": result.total,
                "crit": result.crit, "fumble": result.fumble, "success": result.success,
                "summary": summary, "posted": posted,
            })))
        }
        "dnd5e_adjust_stats" => {
            let actor_arg = str_arg(args, "actor").ok_or_else(|| anyhow!("'actor' is required"))?;
            let requested: Vec<&(&str, &str)> = STAT_PATHS
                .iter()
                .filter(|(k, _)| args.get(*k).is_some())
                .collect();
            if requested.is_empty() {
                bail!(
                    "provide at least one of: {}",
                    STAT_PATHS
                        .iter()
                        .map(|(k, _)| *k)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            let actor = foundry
                .find_document("actors", &actor_arg, None)
                .await?
                .ok_or_else(|| anyhow!("Actor not found: {actor_arg}"))?;
            let absolute = args.get("set").and_then(Value::as_bool).unwrap_or(false);
            let hp_max = get_path(&actor, "system.attributes.hp.max")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX);
            let mut update = serde_json::Map::new();
            let mut changes = serde_json::Map::new();
            for (key, path) in requested {
                let before = get_path(&actor, path).and_then(Value::as_i64).unwrap_or(0);
                let input = args.get(*key).and_then(Value::as_i64).unwrap_or(0);
                let mut after = (if absolute { input } else { before + input }).max(0);
                if *key == "hp" {
                    after = after.min(hp_max);
                }
                update.insert((*path).into(), json!(after));
                changes.insert((*key).into(), json!({"before": before, "after": after}));
            }
            let mut update_doc = Value::Object(update);
            update_doc["_id"] = actor["_id"].clone();
            let result = foundry
                .modify_document(
                    "Actor",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [update_doc],
                    }),
                )
                .await?;
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

    fn seq(vals: &'static [f64]) -> impl FnMut() -> f64 {
        let mut i = 0;
        move || {
            let v = vals[i % vals.len()];
            i += 1;
            v
        }
    }

    #[test]
    fn modificateurs_srd() {
        assert_eq!(ability_modifier(10), 0);
        assert_eq!(ability_modifier(15), 2);
        assert_eq!(ability_modifier(8), -1);
        assert_eq!(proficiency_bonus(1), 2);
        assert_eq!(proficiency_bonus(5), 3);
        assert_eq!(proficiency_bonus(17), 6);
    }

    #[test]
    fn avantage_et_criticals() {
        // 0.2 → 5 ; 0.8 → 17
        assert_eq!(roll_d20(0, true, false, None, seq(&[0.2, 0.8])).kept, 17);
        assert_eq!(roll_d20(0, false, true, None, seq(&[0.2, 0.8])).kept, 5);
        assert_eq!(
            roll_d20(0, true, true, None, seq(&[0.2, 0.8])).rolls.len(),
            1
        );
        let nat20 = roll_d20(-10, false, false, Some(30), seq(&[19.5 / 20.0]));
        assert!(nat20.crit && nat20.success == Some(true));
        let nat1 = roll_d20(50, false, false, Some(5), seq(&[0.0]));
        assert!(nat1.fumble && nat1.success == Some(false));
    }
}
