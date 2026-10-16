//! Garde-fou contre l'exécution de code par la porte des DONNÉES.
//!
//! Le compagnon refuse `client_run_script` tant que le MJ ne l'a pas permis
//! (réglage « Remote script execution »). Mais plusieurs modules exécutent du
//! JavaScript stocké dans des documents ordinaires, que la passerelle — un
//! compte MJ — peut écrire sans passer par le compagnon :
//!
//!   · Monk's Active Tiles : l'action `runcode`, et surtout `eval()` sur les
//!     champs « valeur » (`alter`, positions, comparaisons) et Handlebars avec
//!     accès aux prototypes sur les textes — n'importe quelle tuile est du code
//!     dès qu'un token marche dessus ;
//!   · les comportements de Region du cœur `executeScript` / `executeMacro` ;
//!   · les scripts de presets Mass Edit (`preSpawnScript` / `postSpawnScript`) ;
//!   · les macros attachées aux piles Item Piles.
//!
//! Et les réglages du compagnon eux-mêmes : les passer à « auto » par
//! `set_setting` ouvrait `client_run_script` sans confirmation.
//!
//! Ce module est PUR : il inspecte une opération `modifyDocument` et dit ce
//! qu'il faudrait pour l'accepter. `client.rs` l'appelle pour TOUTE écriture,
//! et lit les réglages du compagnon seulement si c'est nécessaire.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

pub const COMPANION: &str = "foundry-mcp-gateway-companion";

/// Ce qu'une écriture exige pour passer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Need {
    /// Réglage « Remote script execution » du compagnon sur `auto`.
    Script(String),
    /// Réglage « Remote script-macro execution » du compagnon sur `auto`.
    MacroScript(String),
    /// Jamais par la passerelle (les réglages d'exécution du compagnon).
    Forbidden(String),
}

impl Need {
    pub fn reason(&self) -> &str {
        match self {
            Need::Script(r) | Need::MacroScript(r) | Need::Forbidden(r) => r,
        }
    }
}

/// Actions MAT sans exécution de code — à condition que leurs champs passent
/// `check_mat_string`. Tout le reste (runcode, runmacro, runbatch, url,
/// dialog, actions de modules tiers…) exige le mode script automatique.
#[rustfmt::skip]
pub const MAT_SAFE_ACTIONS: &[&str] = &[
    "pause", "delay", "movement", "pancanvas", "ping", "teleport", "movetoken",
    "rotation", "showhide", "activate", "alter", "hurtheal", "playsound",
    "playlist", "stopsound", "showimage", "changedoor", "notification",
    "chatmessage", "rolltable", "resetfog", "openjournal", "openactor",
    "additem", "removeitem", "trigger", "scene", "scenebackground",
    "addtocombat", "elevation", "resethistory", "tileimage", "target",
    "scenelighting", "globalvolume", "gametime", "scrollingtext", "setvariable",
    "activeeffect", "attack", "create", "delete", "preload", "preloadtileimage",
    // filtres et logique
    "anchor", "goto", "loop", "stop", "stoptriggers", "distance", "visibility",
    "exists", "triggercount", "tokencount", "first", "attribute", "inventory",
    "condition", "random", "checkvariable", "checkvalue", "playertype",
    "method", "triggertype",
];

/// Champs de texte libre : jamais évalués comme du JS par MAT, mais compilés
/// par Handlebars (accès aux prototypes autorisé) et rendus en HTML.
#[rustfmt::skip]
const MAT_TEXT_FIELDS: &[&str] = &[
    "text", "flavor", "title", "content", "name", "label", "tag", "audiofile",
    "img", "image", "imagefile", "caption", "language", "message", "fail",
    "none", "yes", "no", "landing", "description", "chatMessage",
];

static SAFE_SCALAR: LazyLock<Regex> = LazyLock::new(|| {
    // « 5 », « -2.5 », « = 5 », « + 3 », « > 0 », « 1-6 », « [[1d6+2]] »,
    // « - [[2d4]] », « true ». Rien qui puisse appeler une fonction.
    Regex::new(
        r"(?i)^\s*(?:==|!=|<=|>=|=|<|>|\+|-)?\s*(?:-?\d+(?:\.\d+)?(?:\s*-\s*\d+(?:\.\d+)?)?|\[\[[0-9dkhlr+\-*/() ]+\]\]|true|false)\s*$",
    )
    .unwrap()
});
static IDENT: LazyLock<Regex> = LazyLock::new(|| {
    // Mots-clés, ids, UUID, listes d'UUID, « tagger:x », chemins de fichiers.
    // Pas d'espace, de parenthèse, de guillemet ni d'opérateur : `eval` d'une
    // telle chaîne ne peut au pire que lire une propriété.
    Regex::new(r"^[A-Za-z0-9_.:#@/,\-]*$").unwrap()
});
static HTML_ACTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<\s*(script|iframe|object|embed)|\bon[a-z]+\s*=|javascript:").unwrap()
});

/// Vérifie une chaîne d'une action MAT. `key` est le nom du champ.
fn check_mat_string(key: &str, s: &str) -> Result<(), String> {
    if s.contains("{{") {
        return Err(format!(
            "« {key} » contains a Handlebars template ({{{{…}}}})"
        ));
    }
    if MAT_TEXT_FIELDS.contains(&key) {
        // Un texte qui commence par un opérateur est évalué par getValue.
        let t = s.trim_start();
        if t.starts_with('=') || t.starts_with("+ ") || t.starts_with("- ") {
            return Err(format!(
                "« {key} » starts with an operator, which MAT evaluates as JavaScript"
            ));
        }
        if HTML_ACTIVE.is_match(s) {
            return Err(format!("« {key} » contains active HTML"));
        }
        return Ok(());
    }
    if SAFE_SCALAR.is_match(s) || IDENT.is_match(s) {
        Ok(())
    } else {
        Err(format!(
            "« {key} » = {s:?} is not a number, a die roll or an identifier — MAT would evaluate it as JavaScript"
        ))
    }
}

fn walk_mat_data(key: &str, v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            if let Err(e) = check_mat_string(key, s) {
                out.push(e);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| walk_mat_data(key, x, out)),
        Value::Object(o) => o.iter().for_each(|(k, x)| walk_mat_data(k, x, out)),
        _ => {}
    }
}

/// Valide une liste d'actions MAT. Renvoie ce qu'il faut pour l'accepter.
pub fn check_mat_actions(actions: &Value) -> Option<Need> {
    let Some(list) = actions.as_array() else {
        return Some(Need::Script("unreadable MAT action list".into()));
    };
    let mut problems = Vec::new();
    let mut macro_needed = false;
    for (i, a) in list.iter().enumerate() {
        let kind = a.get("action").and_then(Value::as_str).unwrap_or("");
        if kind == "runmacro" {
            macro_needed = true;
            continue;
        }
        if !MAT_SAFE_ACTIONS.contains(&kind) {
            problems.push(format!(
                "action #{i} « {kind} » runs code or is not a known safe action"
            ));
            continue;
        }
        let mut errs = Vec::new();
        if let Some(d) = a.get("data") {
            walk_mat_data("data", d, &mut errs);
        }
        problems.extend(
            errs.into_iter()
                .map(|e| format!("action #{i} « {kind} » : {e}")),
        );
    }
    if !problems.is_empty() {
        Some(Need::Script(format!(
            "Monk's Active Tiles — {}",
            problems.join(" ; ")
        )))
    } else if macro_needed {
        Some(Need::MacroScript(
            "Monk's Active Tiles — « runmacro » action".into(),
        ))
    } else {
        None
    }
}

fn non_empty_str(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

/// Parcourt récursivement une donnée de document (flags imbriqués OU clés
/// aplaties « flags.module.x ») à la recherche de contenu exécutable.
fn walk(key: &str, v: &Value, out: &mut Vec<Need>) {
    // Monk's Active Tiles — objet de flags ou clé aplatie.
    if key == "monks-active-tiles" || key.ends_with(".monks-active-tiles") {
        if let Some(actions) = v.get("actions")
            && let Some(n) = check_mat_actions(actions)
        {
            out.push(n);
        }
    } else if key.ends_with("monks-active-tiles.actions") {
        if let Some(n) = check_mat_actions(v) {
            out.push(n);
        }
    } else if key.contains("monks-active-tiles.actions") {
        // Écriture d'un morceau d'action par chemin : impossible à valider seul.
        out.push(Need::Script(format!(
            "partial write of MAT actions ({key}) — write the whole actions list"
        )));
    }

    // Mass Edit — scripts de preset.
    if key.contains("multi-token-edit") && (key.ends_with("SpawnScript")) && non_empty_str(Some(v))
    {
        out.push(Need::Script("Mass Edit — preset spawn script".into()));
    }
    if key == "preset" || key.ends_with(".preset") {
        for s in ["preSpawnScript", "postSpawnScript"] {
            if non_empty_str(v.get(s)) {
                out.push(Need::Script(format!("Mass Edit — {s}")));
            }
        }
    }

    // Item Piles — macro déclenchée par les interactions avec une pile/un objet.
    if (key == "item-piles" || key.contains("item-piles")) && v.is_object() {
        for sub in ["data", "item"] {
            if non_empty_str(v.get(sub).and_then(|d| d.get("macro"))) {
                out.push(Need::MacroScript("Item Piles — pile macro".into()));
            }
        }
    }
    if key.contains("item-piles") && key.ends_with(".macro") && non_empty_str(Some(v)) {
        out.push(Need::MacroScript("Item Piles — pile macro".into()));
    }

    // Region behaviors du cœur (incrustés dans une Scene ou une Region).
    if let Some(t) = v.get("type").and_then(Value::as_str)
        && v.get("system").is_some()
    {
        match t {
            "executeScript" => out.push(Need::Script("« executeScript » Region behavior".into())),
            "executeMacro" => {
                out.push(Need::MacroScript("« executeMacro » Region behavior".into()))
            }
            _ => {}
        }
    }

    match v {
        Value::Object(o) => o.iter().for_each(|(k, x)| walk(k, x, out)),
        Value::Array(a) => a.iter().for_each(|x| walk(key, x, out)),
        _ => {}
    }
}

/// Inspecte une opération create/update. `setting_keys` : pour une mise à
/// jour de Setting, les clés des documents visés (résolues par l'appelant,
/// l'update ne porte que des `_id`).
pub fn inspect(
    doc_type: &str,
    action: &str,
    operation: &Value,
    setting_keys: &[String],
) -> Vec<Need> {
    let mut out = Vec::new();
    if action != "create" && action != "update" {
        return out;
    }
    let items = operation
        .get(if action == "create" {
            "data"
        } else {
            "updates"
        })
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if doc_type == "Setting" {
        let mut keys: Vec<String> = items
            .iter()
            .filter_map(|d| d.get("key").and_then(Value::as_str).map(String::from))
            .collect();
        keys.extend(setting_keys.iter().cloned());
        for k in keys {
            if let Some(rest) = k.strip_prefix(&format!("{COMPANION}."))
                && rest != "rootPublicKey"
            {
                out.push(Need::Forbidden(format!(
                    "the companion setting « {k} » can only be changed in Foundry, by the GM"
                )));
            }
        }
        return out;
    }
    if doc_type == "RegionBehavior" {
        for d in &items {
            match d.get("type").and_then(Value::as_str) {
                Some("executeScript") => {
                    out.push(Need::Script("« executeScript » Region behavior".into()))
                }
                Some("executeMacro") => {
                    out.push(Need::MacroScript("« executeMacro » Region behavior".into()))
                }
                _ => {}
            }
        }
    }
    for d in &items {
        walk("", d, &mut out);
    }
    out.dedup();
    out
}

/// Message d'erreur commun : dit ce qui a été vu ET comment l'autoriser.
pub fn refusal(need: &Need, script_mode: &str, macro_mode: &str) -> String {
    match need {
        Need::Forbidden(r) => format!("refused: {r}."),
        Need::Script(r) => format!(
            "refused: {r}. Foundry would run this as JavaScript in the GM's browser, which bypasses \
             the companion's « Remote script execution » setting (currently « {script_mode} »). \
             Only « auto » allows it — a choice the GM makes in Foundry, not through the gateway."
        ),
        Need::MacroScript(r) => format!(
            "refused: {r}. It runs a macro outside the companion's « Remote script-macro execution » \
             setting (currently « {macro_mode} »). Only « auto » allows it — a choice the GM makes in Foundry."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tile(actions: Value) -> Value {
        json!({"action":"create","data":[{"x":0,"flags":{"monks-active-tiles":{"active":true,"actions":actions}}}]})
    }

    #[test]
    fn tuile_inoffensive_acceptee() {
        let op = tile(json!([
            {"id":"a","action":"pause","data":{"pause":"pause"}},
            {"id":"b","action":"chatmessage","data":{"text":"Un piège s'ouvre !","showto":"everyone","chatbubble":"false"}},
            {"id":"c","action":"hurtheal","data":{"entity":{"id":"token"},"value":"- [[2d6]]"}},
            {"id":"d","action":"alter","data":{"entity":{"id":"Scene.a.Token.b"},"attribute":"system.attributes.hp.value","value":"= 5"}},
            {"id":"e","action":"teleport","data":{"location":{"id":"Scene.a.Tile.c","x":"+ 100","sceneId":"a"}}}
        ]));
        assert!(inspect("Tile", "create", &op, &[]).is_empty());
    }

    #[test]
    fn runcode_et_eval_refuses() {
        let op = tile(json!([{"id":"a","action":"runcode","data":{"code":"alert(1)"}}]));
        assert!(matches!(
            inspect("Tile", "create", &op, &[])[0],
            Need::Script(_)
        ));
        let op = tile(
            json!([{"id":"a","action":"alter","data":{"attribute":"x","value":"= fetch('//e.vil')"}}]),
        );
        assert!(matches!(
            inspect("Tile", "create", &op, &[])[0],
            Need::Script(_)
        ));
        let op = tile(
            json!([{"id":"a","action":"chatmessage","data":{"text":"{{#with this}}{{/with}}"}}]),
        );
        assert!(matches!(
            inspect("Tile", "create", &op, &[])[0],
            Need::Script(_)
        ));
        let op = tile(
            json!([{"id":"a","action":"notification","data":{"text":"<img src=x onerror=alert(1)>"}}]),
        );
        assert!(matches!(
            inspect("Tile", "create", &op, &[])[0],
            Need::Script(_)
        ));
    }

    #[test]
    fn runmacro_exige_le_mode_macro() {
        let op = tile(json!([{"id":"a","action":"runmacro","data":{"entity":{"id":"Macro.x"}}}]));
        assert!(matches!(
            inspect("Tile", "create", &op, &[])[0],
            Need::MacroScript(_)
        ));
    }

    #[test]
    fn cles_aplaties_et_scenes_incrustees() {
        let op = json!({"action":"update","updates":[{"_id":"t","flags.monks-active-tiles.actions":[{"action":"runcode","data":{}}]}]});
        assert!(!inspect("Tile", "update", &op, &[]).is_empty());
        let op = json!({"action":"update","updates":[{"_id":"t","flags.monks-active-tiles.actions.0.data.value":"1"}]});
        assert!(!inspect("Tile", "update", &op, &[]).is_empty());
        // Activer/désactiver une tuile ne touche pas aux actions : libre.
        let op = json!({"action":"update","updates":[{"_id":"t","flags.monks-active-tiles.active":false}]});
        assert!(inspect("Tile", "update", &op, &[]).is_empty());
        // Une scène qui embarque une tuile piégée.
        let op = json!({"action":"create","data":[{"name":"S","tiles":[{"flags":{"monks-active-tiles":{"actions":[{"action":"runcode"}]}}}]}]});
        assert!(!inspect("Scene", "create", &op, &[]).is_empty());
    }

    #[test]
    fn regions_presets_et_piles() {
        let op =
            json!({"action":"create","data":[{"type":"executeScript","system":{"source":"x"}}]});
        assert!(matches!(
            inspect("RegionBehavior", "create", &op, &[])[0],
            Need::Script(_)
        ));
        let op = json!({"action":"create","data":[{"name":"R","behaviors":[{"type":"executeMacro","system":{"uuid":"Macro.x"}}]}]});
        assert!(matches!(
            inspect("Region", "create", &op, &[])[0],
            Need::MacroScript(_)
        ));
        let op = json!({"action":"create","data":[{"flags":{"multi-token-edit":{"preset":{"postSpawnScript":"x()"}}}}]});
        assert!(!inspect("JournalEntry", "create", &op, &[]).is_empty());
        let op = json!({"action":"create","data":[{"flags":{"multi-token-edit":{"preset":{"postSpawnScript":""}}}}]});
        assert!(inspect("JournalEntry", "create", &op, &[]).is_empty());
        let op = json!({"action":"create","data":[{"flags":{"item-piles":{"data":{"enabled":true,"macro":"Macro.x"}}}}]});
        assert!(matches!(
            inspect("Actor", "create", &op, &[])[0],
            Need::MacroScript(_)
        ));
    }

    #[test]
    fn reglages_du_compagnon_interdits_sauf_la_cle() {
        let op = json!({"action":"create","data":[{"key":"foundry-mcp-gateway-companion.runScriptMode","value":"\"auto\""}]});
        assert!(matches!(
            inspect("Setting", "create", &op, &[])[0],
            Need::Forbidden(_)
        ));
        let op = json!({"action":"update","updates":[{"_id":"s","value":"\"auto\""}]});
        let keys = vec!["foundry-mcp-gateway-companion.runMacroScriptMode".to_string()];
        assert!(matches!(
            inspect("Setting", "update", &op, &keys)[0],
            Need::Forbidden(_)
        ));
        let keys = vec!["foundry-mcp-gateway-companion.rootPublicKey".to_string()];
        assert!(inspect("Setting", "update", &op, &keys).is_empty());
        let keys = vec!["core.time".to_string()];
        assert!(inspect("Setting", "update", &op, &keys).is_empty());
    }
}
