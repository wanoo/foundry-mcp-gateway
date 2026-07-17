//! Dérivation starwarsffg : valeurs affichées = stockées + mods `attributes`
//! (acteur, items, talents APPRIS des spécialisations, upgrades appris des
//! pouvoirs de la Force). Résout le piège « le doc source affiche 0 ».

use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct AttributeMod {
    pub target: String,
    pub modtype: String,
    pub value: i64,
    pub source: String,
}

fn read_attributes(attrs: Option<&Value>, source: &str, out: &mut Vec<AttributeMod>) {
    let Some(map) = attrs.and_then(Value::as_object) else { return };
    for entry in map.values() {
        let (Some(target), Some(modtype)) = (
            entry.get("mod").and_then(Value::as_str),
            entry.get("modtype").and_then(Value::as_str),
        ) else { continue };
        let value = entry.get("value").and_then(Value::as_i64)
            .or_else(|| entry.get("value").and_then(Value::as_bool).map(|b| b as i64))
            .unwrap_or(0);
        if value == 0 { continue; }
        out.push(AttributeMod {
            target: target.to_string(),
            modtype: modtype.to_string(),
            value,
            source: source.to_string(),
        });
    }
}

pub fn collect_attribute_mods(actor: &Value) -> Vec<AttributeMod> {
    let mut mods = Vec::new();
    read_attributes(actor.pointer("/system/attributes"), "acteur", &mut mods);
    let empty = vec![];
    for item in actor.get("items").and_then(Value::as_array).unwrap_or(&empty) {
        let name = item.get("name").and_then(Value::as_str)
            .or_else(|| item.get("type").and_then(Value::as_str)).unwrap_or("item");
        read_attributes(item.pointer("/system/attributes"), name, &mut mods);
        match item.get("type").and_then(Value::as_str) {
            Some("specialization") => {
                if let Some(talents) = item.pointer("/system/talents").and_then(Value::as_object) {
                    for t in talents.values() {
                        if t.get("islearned").and_then(Value::as_bool).unwrap_or(false) {
                            let tname = format!("talent {}", t.get("name").and_then(Value::as_str).unwrap_or("?"));
                            read_attributes(t.get("attributes"), &tname, &mut mods);
                        }
                    }
                }
            }
            Some("forcepower") => {
                if let Some(sys) = item.get("system").and_then(Value::as_object) {
                    for (key, u) in sys {
                        if key.starts_with("upgrade")
                            && u.get("islearned").and_then(Value::as_bool).unwrap_or(false)
                        {
                            let uname = format!("pouvoir {}", u.get("name").and_then(Value::as_str).unwrap_or(key));
                            read_attributes(u.get("attributes"), &uname, &mut mods);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    mods
}

fn sum_mods(mods: &[AttributeMod], modtype: &str, target: &str) -> i64 {
    mods.iter().filter(|m| m.modtype == modtype && m.target == target).map(|m| m.value).sum()
}

pub fn derive_characteristic(actor: &Value, name: &str, mods: &[AttributeMod]) -> i64 {
    let stored = actor
        .pointer(&format!("/system/characteristics/{name}/value"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    stored + sum_mods(mods, "Characteristic", name)
}

/// Pool dérivé d'une compétence : jaunes = min(carac, rang), verts = le reste,
/// + boosts / setbacks / remove-setbacks des talents.
pub fn derive_skill_pool(actor: &Value, skill_name: &str) -> Option<Value> {
    let skills = actor.pointer("/system/skills")?.as_object()?;
    let key = skills.keys().find(|k| k.to_lowercase() == skill_name.to_lowercase())?.clone();
    let skill = &skills[&key];
    let characteristic = skill.get("characteristic").and_then(Value::as_str).unwrap_or("Brawn").to_string();

    let mods = collect_attribute_mods(actor);
    let char_value = derive_characteristic(actor, &characteristic, &mods).max(0);
    let rank = (skill.get("rank").and_then(Value::as_i64).unwrap_or(0)
        + sum_mods(&mods, "Skill Rank", &key)).max(0);
    let proficiency = char_value.min(rank);
    let ability = char_value.max(rank) - proficiency;
    let sources: Vec<String> = mods.iter()
        .filter(|m| m.target == key || (m.modtype == "Characteristic" && m.target == characteristic))
        .map(|m| format!("{} ({} {:+})", m.source, m.modtype, m.value))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter().collect();

    Some(json!({
        "skill": key,
        "characteristic": characteristic,
        "characteristicValue": char_value,
        "rank": rank,
        "proficiency": proficiency,
        "ability": ability,
        "boost": sum_mods(&mods, "Skill Boost", &key).max(0),
        "setback": sum_mods(&mods, "Skill Setback", &key).max(0),
        "removeSetback": sum_mods(&mods, "Skill Remove Setback", &key).max(0),
        "sources": sources,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor() -> Value {
        json!({
            "system": {
                "characteristics": {"Willpower": {"value": 0}, "Cunning": {"value": 0}},
                "skills": {
                    "Vigilance": {"rank": 0, "characteristic": "Willpower"},
                    "Perception": {"rank": 1, "characteristic": "Cunning"},
                },
                "attributes": {"achat": {"mod": "Willpower", "modtype": "Characteristic", "value": 1}},
            },
            "items": [
                {"name": "Twi'lek", "type": "species", "system": {"attributes": {
                    "W": {"mod": "Willpower", "modtype": "Characteristic", "value": 2},
                    "C": {"mod": "Cunning", "modtype": "Characteristic", "value": 2},
                }}},
                {"name": "Visionnaire", "type": "specialization", "system": {"talents": {
                    "t1": {"name": "Réaction fulgurante", "islearned": true, "attributes": {
                        "a": {"mod": "Vigilance", "modtype": "Skill Boost", "value": 1},
                        "b": {"mod": "Vigilance", "modtype": "Skill Boost", "value": 1},
                    }},
                    "t2": {"name": "Pas appris", "islearned": false, "attributes": {
                        "x": {"mod": "Vigilance", "modtype": "Skill Boost", "value": 5},
                    }},
                    "t3": {"name": "Maîtrise", "islearned": true, "attributes": {
                        "r": {"mod": "Perception", "modtype": "Skill Rank", "value": 1},
                    }},
                }}},
            ],
        })
    }

    #[test]
    fn caracteristique_derivee() {
        let a = actor();
        let mods = collect_attribute_mods(&a);
        assert_eq!(derive_characteristic(&a, "Willpower", &mods), 3); // 0 + 2 espèce + 1 achat
        assert_eq!(derive_characteristic(&a, "Cunning", &mods), 2);
        assert!(mods.iter().all(|m| !m.source.contains("Pas appris")));
    }

    #[test]
    fn pool_de_competence() {
        let a = actor();
        let p = derive_skill_pool(&a, "vigilance").unwrap(); // insensible à la casse
        assert_eq!(p["characteristicValue"], 3);
        assert_eq!(p["ability"], 3);
        assert_eq!(p["proficiency"], 0);
        assert_eq!(p["boost"], 2);
        let p2 = derive_skill_pool(&a, "Perception").unwrap();
        assert_eq!(p2["rank"], 2); // 1 stocké + 1 talent
        assert_eq!(p2["proficiency"], 2);
        assert!(derive_skill_pool(&a, "Basket").is_none());
    }
}
