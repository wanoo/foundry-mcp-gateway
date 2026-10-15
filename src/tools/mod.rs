//! Registre d'outils MCP — noyau générique. (Portage progressif : ce module
//! couvre lectures/écritures/packs/recherche/événements ; les outils de séance
//! et les modules système arrivent par lots.)

pub mod addons;
pub mod admin;
pub mod cc_family;
pub mod companion;
pub mod generic;
pub mod insight;
pub mod manage;
pub mod markdown;
pub mod session;
pub mod table;
pub mod transfer;

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::mcp::McpState;
use crate::systems;

/// Définition d'un outil : (nom, description, schéma JSON des arguments).
///
/// `&'static str` : ce sont des littéraux du code, jamais dérivés de l'état —
/// ce qui permet de les construire une seule fois par module (voir les `DEFS`
/// paresseux) au lieu de rebâtir les schémas à chaque dispatch.
pub type ToolDef = (&'static str, &'static str, Value);

pub const COLLECTIONS: [(&str, &str, &str); 13] = [
    ("actors", "actor", "actor"),
    ("items", "item", "item"),
    ("folders", "folder", "folder"),
    ("users", "user", "user"),
    ("scenes", "scene", "scene"),
    ("journals", "journal", "journal entry"),
    ("macros", "macro", "macro"),
    ("cards", "card", "cards stack"),
    ("playlists", "playlist", "playlist"),
    ("tables", "table", "roll table"),
    ("combats", "combat", "combat"),
    ("messages", "message", "chat message"),
    ("settings", "setting", "setting"),
];

/// pluriel d'outil → nom de collection Foundry.
pub fn plural_to_collection(plural: &str) -> &str {
    if plural == "journals" {
        "journal"
    } else {
        plural
    }
}

pub fn text_response(value: &Value) -> Value {
    let mut out = json!({ "content": [{ "type": "text", "text": value.to_string() }] });
    // MCP 2025-06-18 : les clients récents lisent structuredContent (objets
    // seulement, par spec) ; le texte reste pour les clients plus anciens.
    if value.is_object() {
        out["structuredContent"] = value.clone();
    }
    out
}

/// Contenu image MCP natif (base64 brut, sans préfixe data:) + une légende texte.
pub fn image_response(base64: &str, mime_type: &str, caption: &Value) -> Value {
    json!({ "content": [
        { "type": "image", "data": base64, "mimeType": mime_type },
        { "type": "text", "text": caption.to_string() },
    ]})
}

pub fn error_response(message: String) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

/// Poste un ChatMessage (author = le bot, whisper optionnel) — helper partagé.
pub async fn post_chat(
    state: &McpState,
    content: &str,
    flags: Value,
    whisper: Option<&Value>,
) -> Result<Value> {
    let mut message = json!({
        "content": content,
        "author": state.foundry.user_id().await,
        "flags": flags,
    });
    if let Some(w) = whisper.and_then(Value::as_array)
        && !w.is_empty()
    {
        message["whisper"] = json!(w);
    }
    state
        .foundry
        .modify_document(
            "ChatMessage",
            "create",
            json!({
                "action": "create", "renderSheet": false, "keepId": false,
                "data": [message],
            }),
        )
        .await
}

/// Tirages sur une RollTable (formule NdM±k + modificateur, sélection par plage).
///
/// Chaque tirage porte aussi, si le résultat touché en a : `quantity` et
/// `quantityFormula` (Mini Rolltable) et `linkedTable` (résultat qui pointe
/// une autre RollTable). Sans ces données, la sortie est inchangée.
pub fn roll_table_draws(table: &Value, modifier: i64, rolls: usize) -> Result<Vec<Value>> {
    let formula = table
        .get("formula")
        .and_then(Value::as_str)
        .unwrap_or("1d100")
        .replace(char::is_whitespace, "");
    let re = regex::Regex::new(r"^(?i)(\d+)d(\d+)([+-]\d+)?$").unwrap();
    let caps = re
        .captures(&formula)
        .ok_or_else(|| anyhow!("Unsupported table formula '{formula}' (expected NdM±k)"))?;
    let n: i64 = caps[1].parse()?;
    let m: i64 = caps[2].parse()?;
    // La formule vient de la RollTable du MONDE, pas de l'appelant : Foundry
    // accepte « 1d0 » à la création, et `% m` panique dessus. `n` non borné
    // bloquerait un worker tokio pour l'éternité. On cite la formule fautive
    // parce que apply_critical_injury emprunte ce chemin avec une table
    // AUTO-DÉTECTÉE par nom : sans ça, le MJ n'a aucun moyen de la retrouver.
    if !(1..=1000).contains(&m) {
        bail!("Unsupported table formula '{formula}': die size must be between 1 and 1000");
    }
    if !(1..=100).contains(&n) {
        bail!("Unsupported table formula '{formula}': dice count must be between 1 and 100");
    }
    let k: i64 = caps
        .get(3)
        .map(|c| c.as_str().parse().unwrap_or(0))
        .unwrap_or(0);
    let empty = vec![];
    let results = table
        .get("results")
        .and_then(Value::as_array)
        .unwrap_or(&empty);

    let mut draws = Vec::new();
    for _ in 0..rolls {
        let mut roll = k + modifier;
        for _ in 0..n {
            roll += 1 + (rand::random::<f64>() * m as f64) as i64 % m;
        }
        let hit = results.iter().find(|r| {
            r.get("range")
                .and_then(Value::as_array)
                .and_then(|rg| Some((rg.first()?.as_i64()?, rg.get(1)?.as_i64()?)))
                .map(|(lo, hi)| roll >= lo && roll <= hi)
                .unwrap_or(false)
        });
        // v13+ : un résultat « document » a souvent une description VIDE et
        // son libellé dans `name` — on prend le premier champ non vide.
        let text = hit.and_then(|h| {
            ["description", "text", "name"]
                .iter()
                .filter_map(|k| h.get(*k).and_then(Value::as_str))
                .find(|t| !t.trim().is_empty())
        });
        let mut draw = json!({
            "roll": roll,
            "range": hit.and_then(|h| h.get("range")).cloned(),
            "text": text,
            "documentUuid": hit.and_then(|h| h.get("documentUuid")).cloned(),
        });
        // Mini Rolltable : quantité par résultat (flags["mini-rolltable"].quantity)
        // et lien vers une autre table (suivi par draw_from_table, qui charge
        // la table liée — ici on reste pur, sans accès au monde).
        if let Some(h) = hit {
            if let Some(f) = cc_family::mini_suite::mrt_quantity_formula(h) {
                let (q, warn) = cc_family::mini_suite::mrt_roll_quantity(&f, rand::random::<f64>);
                draw["quantityFormula"] = json!(f);
                draw["quantity"] = json!(q);
                if let Some(w) = warn {
                    draw["warning"] = json!(w);
                }
            }
            if let Some((uuid, id)) = cc_family::mini_suite::mrt_linked_table(h) {
                draw["linkedTable"] = json!({"uuid": uuid, "_id": id});
            }
        }
        draws.push(draw);
    }
    Ok(draws)
}

/// Troncature par taille JSON (compat `max_length` du serveur TS).
///
/// Une seule passe : la longueur de `[a,b,c]` vaut 2 + Σ len(item) + les
/// virgules, donc un cumul suffit. L'implémentation précédente clonait TOUT le
/// vecteur et le re-sérialisait à chaque document abandonné — quadratique, et
/// c'était le point chaud d'un `get_actors` avec `max_length` sur un monde
/// chargé. Sémantique inchangée : on ne coupe jamais au milieu d'un document.
pub(super) fn truncate_by_bytes(mut docs: Vec<Value>, max_length: Option<usize>) -> Vec<Value> {
    let Some(max) = max_length.filter(|m| *m > 0) else {
        return docs;
    };
    let mut total = 2; // les crochets
    let mut kept = 0;
    for d in &docs {
        let len = d.to_string().len() + usize::from(kept > 0); // virgule séparatrice
        if total + len > max {
            break;
        }
        total += len;
        kept += 1;
    }
    docs.truncate(kept);
    docs
}

/// Un outil sans effet de bord sur le monde (sert aux annotations ET au mode
/// FOUNDRY_READONLY). ⚠️ manage_modules/manage_users ÉCRIVENT : jamais ici.
pub fn is_read_only(name: &str) -> bool {
    cc_family::is_read_only(name)
        || name.starts_with("get_")
        || name.starts_with("list_")
        || matches!(
            name,
            "admin_status"
                | "admin_check_package"
                | "admin_list_backups"
                | "search_journals"
                | "ping"
                | "export_journals"
                | "client_status"
                | "client_get_state"
                | "client_get_derived"
                | "client_enrich"
                | "client_search"
                | "client_capture"
                | "client_scene_report"
                | "client_babele"
                | "client_weather_types"
                | "client_token_fx_presets"
                | "client_effect_catalog"
                // Ces huit-là manquaient : ils ne touchent à rien mais étaient
                // refusés en mode FOUNDRY_READONLY, qui était donc plus strict
                // que ce que la doc annonce. La liste est manuelle et a déjà
                // dérivé ; `liste_lecture_seule_sans_fantome` attrape au moins
                // le cas d'un nom qui survit à un outil renommé ou supprimé.
                | "browse_files"
                | "show_credentials"
                | "mat_list"
        )
}

fn annotations(name: &str) -> Value {
    let destructive = matches!(name, "delete_document" | "delete_compendium");
    json!({ "readOnlyHint": is_read_only(name), "destructiveHint": destructive })
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    // Chaque outil accepte `instance` : le monde visé (voir show_credentials).
    let mut schema = schema;
    if !matches!(
        name,
        "copy_documents" | "show_credentials" | "choose_foundry_instance"
    ) && let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut)
    {
        props.insert(
            "instance".to_string(),
            json!({"type":"string","description":"target Foundry instance _id (default: the active one) — several worlds can be served at once"}),
        );
    }
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
        "annotations": annotations(name),
    })
}

pub fn definitions(state: &McpState) -> Vec<Value> {
    let mut tools = Vec::new();
    // (mode lecture seule : filtrage en fin de fonction)
    for (plural, singular, label) in COLLECTIONS {
        tools.push(tool(
            &format!("get_{plural}"),
            &format!("Get all {label}s. Filter with `where` (dotted paths + __in/__contains/__ne/__exists), project with requested_fields, paginate with offset/limit."),
            json!({ "type": "object", "properties": {
                "where": { "type": "object", "additionalProperties": true },
                "requested_fields": { "type": "array", "items": { "type": "string" } },
                "offset": { "type": "number" },
                "limit": { "type": "number" },
                "max_length": { "type": "number", "description": "Truncate the response to ~this many JSON bytes (drops trailing docs)" },
            }}),
        ));
        tools.push(tool(
            &format!("get_{singular}"),
            &format!("Get one {label} by _id or name."),
            json!({ "type": "object", "properties": {
                "_id": { "type": "string" },
                "id": { "type": "string" },
                "name": { "type": "string" },
                "requested_fields": { "type": "array", "items": { "type": "string" } },
            }}),
        ));
    }
    tools.push(tool(
        "ping",
        "Lightweight health check: Foundry /api/status + connection state (no world dump).",
        json!({ "type": "object", "properties": {} }),
    ));
    tools.push(tool(
        "get_world",
        "World metadata (title, system, modules) — the only heavy full-dump call; prefer ping for liveness.",
        json!({ "type": "object", "properties": {} }),
    ));
    tools.push(tool(
        "get_current_scene",
        "The currently active scene.",
        json!({ "type": "object", "properties": {
            "requested_fields": { "type": "array", "items": { "type": "string" } },
        }}),
    ));
    tools.push(tool(
        "search_journals",
        "Full-text search across journal names and page contents (HTML stripped, case-insensitive).",
        json!({ "type": "object", "properties": {
            "query": { "type": "string" },
            "max_results": { "type": "number" },
        }, "required": ["query"] }),
    ));
    tools.push(tool(
        "list_compendium_packs",
        "List the world's compendium packs (id, label, type).",
        json!({ "type": "object", "properties": {} }),
    ));
    tools.push(tool(
        "get_pack_documents",
        "Read documents from a compendium pack (index auto for _id/name listings).",
        json!({ "type": "object", "properties": {
            "type": { "type": "string" },
            "pack": { "type": "string" },
            "query": { "type": "object", "additionalProperties": true },
            "requested_fields": { "type": "array", "items": { "type": "string" } },
            "max_length": { "type": "number" },
        }, "required": ["type", "pack"] }),
    ));
    tools.push(tool(
        "create_document",
        "Create documents (data = ARRAY even for one). parent_uuid for embedded, pack for compendia, keep_id to preserve _ids.",
        json!({ "type": "object", "properties": {
            "type": { "type": "string" },
            "data": { "type": "array", "items": { "type": "object" } },
            "parent_uuid": { "type": "string" },
            "pack": { "type": "string" },
            "keep_id": { "type": "boolean" },
        }, "required": ["type", "data"] }),
    ));
    tools.push(tool(
        "modify_document",
        "Update a document (dotted-key updates merge into the document).",
        json!({ "type": "object", "properties": {
            "type": { "type": "string" },
            "_id": { "type": "string" },
            "updates": { "type": "array", "items": { "type": "object" } },
            "parent_uuid": { "type": "string" },
            "pack": { "type": "string" },
        }, "required": ["type", "_id", "updates"] }),
    ));
    tools.push(tool(
        "delete_document",
        "Delete documents by ids (ARRAY). Permanent.",
        json!({ "type": "object", "properties": {
            "type": { "type": "string" },
            "ids": { "type": "array", "items": { "type": "string" } },
            "parent_uuid": { "type": "string" },
            "pack": { "type": "string" },
        }, "required": ["type", "ids"] }),
    ));
    tools.push(tool(
        "get_events",
        "Buffered socket broadcasts from OTHER clients (chat, writes, combat). Poll incrementally with since_seq.",
        json!({ "type": "object", "properties": {
            "since_seq": { "type": "number" },
            "event": { "type": "string" },
            "limit": { "type": "number" },
        }}),
    ));
    // Outils de séance, gestion, Campaign Codex, et modules système.
    let admin_defs = admin::definitions(state);
    let system_defs: Vec<&ToolDef> = systems::loaded_modules()
        .iter()
        .flat_map(|m| (m.definitions)().iter())
        .collect();
    for (name, desc, schema) in session::definitions()
        .iter()
        .chain(manage::definitions())
        .chain(cc_family::definitions())
        .chain(addons::definitions())
        .chain(insight::definitions())
        .chain(table::definitions())
        .chain(admin_defs.iter().copied())
        .chain(transfer::definitions())
        .chain(companion::definitions())
        .chain(system_defs)
    {
        // Un clone du schéma par appel à tools/list — rare, et le prix à payer
        // pour que `handles()` ne reconstruise plus rien sur le chemin chaud.
        tools.push(tool(name, desc, schema.clone()));
    }
    if state.readonly {
        tools.retain(|t| {
            t.get("name")
                .and_then(Value::as_str)
                .is_some_and(is_read_only)
        });
    }
    tools
}

/// Recopie les clés PRÉSENTES d'un Value vers un autre.
///
/// Le patron des proxys vers le compagnon : les arguments optionnels ne sont
/// transmis que s'ils ont été fournis, pour que le module navigateur applique
/// ses propres valeurs par défaut. Était réécrit en boucle inline dans cinq
/// modules ; une seule définition évite qu'ils divergent.
pub fn carry(args: &Value, keys: &[&str], out: &mut Value) {
    for k in keys {
        if let Some(v) = args.get(*k) {
            out[*k] = v.clone();
        }
    }
}

pub fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(String::from)
}

pub fn fields_arg(args: &Value) -> Option<Vec<String>> {
    args.get("requested_fields")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
}

pub fn where_arg(args: &Value) -> Option<Map<String, Value>> {
    args.get("where").and_then(Value::as_object).cloned()
}

pub async fn dispatch(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    // Multi-mondes : `instance` choisit la connexion Foundry visée. Comme tous
    // les outils passent par `state.foundry`, il suffit de la substituer ici.
    let routed;
    let state = match args.get("instance").and_then(Value::as_str) {
        Some(inst) => match state.resolve(Some(inst)).await {
            Ok(s) => {
                routed = s;
                &routed
            }
            Err(e) => return Ok(error_response(format!("Error: {e:#}"))),
        },
        None => state,
    };
    if state.readonly && !is_read_only(name) {
        return Ok(error_response(format!(
            "Error: '{name}' modifies the world and this gateway runs in read-only mode \
             (FOUNDRY_READONLY). Only read-only tools are available."
        )));
    }
    let result = if session::handles(name) {
        session::run(state, name, args).await
    } else if manage::handles(name) {
        manage::run(state, name, args).await
    } else if cc_family::handles(name) {
        cc_family::run(state, name, args).await
    } else if addons::handles(name) {
        addons::run(state, name, args).await
    } else if transfer::handles(name) {
        transfer::run(state, name, args).await
    } else if admin::handles(name) {
        admin::run(state, name, args).await
    } else if table::handles(name) {
        table::run(state, name, args).await
    } else if insight::handles(name) {
        insight::run(state, name, args).await
    } else if companion::handles(name) {
        companion::run(state, name, args).await
    } else if systems::loaded_modules().iter().any(|m| (m.handles)(name)) {
        systems::run(state, name, args).await
    } else {
        generic::run_tool(state, name, args).await
    };
    match result {
        Ok(v) => Ok(v),
        Err(e) => Ok(error_response(format!("Error: {e:#}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une table à trois plages contiguës, comme en produit Foundry.
    fn table(formula: &str) -> Value {
        json!({
            "formula": formula,
            "results": [
                {"range": [1, 1], "text": "un"},
                {"range": [2, 2], "text": "deux"},
                {"range": [3, 3], "text": "trois"},
            ]
        })
    }

    /// Tous les modules qui revendiquent des noms, dans l'ordre du dispatch.
    /// (id du module, prédicat d'appartenance).
    type Owner = (&'static str, fn(&str) -> bool);

    fn owners() -> Vec<Owner> {
        let mut v: Vec<Owner> = vec![
            ("session", session::handles),
            ("manage", manage::handles),
            ("cc_family", cc_family::handles),
            ("addons", addons::handles),
            ("insight", insight::handles),
            ("table", table::handles),
            ("admin", admin::handles),
            ("transfer", transfer::handles),
            ("companion", companion::handles),
        ];
        v.extend(systems::all_modules().iter().map(|m| (m.id, m.handles)));
        v
    }

    /// Le vrai livrable du lot : chaque outil appartient à exactement un module.
    ///
    /// À zéro propriétaire, l'outil est annoncé dans `tools/list` puis répond
    /// « Unknown tool ». À deux, le gagnant dépend de l'ordre des `else if` du
    /// dispatch — c'est exactement ce qui « protégeait » companion.rs par
    /// accident, avec son `starts_with("client_")` qui revendiquait 40 outils
    /// pour onze traités.
    #[test]
    fn chaque_outil_a_un_seul_proprietaire() {
        let mut noms: Vec<&'static str> = Vec::new();
        for (_, defs) in [
            ("session", session::definitions()),
            ("manage", manage::definitions()),
            ("cc_family", cc_family::definitions()),
            ("addons", addons::definitions()),
            ("insight", insight::definitions()),
            ("table", table::definitions()),
            ("transfer", transfer::definitions()),
            ("companion", companion::definitions()),
        ] {
            noms.extend(defs.iter().map(|(n, _, _)| *n));
        }
        noms.extend(admin::DEFS_FOR_TEST.iter().map(|(n, _, _)| *n));
        for m in systems::all_modules() {
            noms.extend((m.definitions)().iter().map(|(n, _, _)| *n));
        }

        for nom in &noms {
            let proprietaires: Vec<&str> = owners()
                .iter()
                .filter(|(_, h)| h(nom))
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(
                proprietaires.len(),
                1,
                "l'outil « {nom} » a {} propriétaires : {proprietaires:?}",
                proprietaires.len()
            );
        }

        // Et les outils générés depuis COLLECTIONS ne doivent être revendiqués
        // par PERSONNE : ils tombent volontairement dans run_tool.
        for (plural, singular, _) in COLLECTIONS {
            for nom in [format!("get_{plural}"), format!("get_{singular}")] {
                let revendique: Vec<&str> = owners()
                    .iter()
                    .filter(|(_, h)| h(&nom))
                    .map(|(id, _)| *id)
                    .collect();
                assert!(
                    revendique.is_empty(),
                    "« {nom} » est générique mais revendiqué par {revendique:?}"
                );
            }
        }
    }

    #[test]
    fn liste_lecture_seule_sans_fantome() {
        // Chaque nom cité en dur dans is_read_only doit exister : la liste est
        // manuelle, et un outil renommé y laisserait un fantôme silencieux.
        let mut noms: Vec<&'static str> = Vec::new();
        for defs in [
            session::definitions(),
            manage::definitions(),
            cc_family::definitions(),
            addons::definitions(),
            insight::definitions(),
            table::definitions(),
            transfer::definitions(),
            companion::definitions(),
        ] {
            noms.extend(defs.iter().map(|(n, _, _)| *n));
        }
        noms.extend(admin::DEFS_FOR_TEST.iter().map(|(n, _, _)| *n));
        for nom in [
            "admin_status",
            "admin_check_package",
            "admin_list_backups",
            "search_journals",
            "ping",
            "export_journals",
            "client_status",
            "client_get_state",
            "client_get_derived",
            "client_enrich",
            "client_search",
            "client_capture",
            "client_scene_report",
            "client_babele",
            "client_weather_types",
            "client_token_fx_presets",
            "client_effect_catalog",
            "browse_files",
            "show_credentials",
            "cc_list_sheets",
            "cc_get_sheet",
            "al_find",
            "mc_get_time",
            "mc_list_notes",
            "mat_list",
        ] {
            // `ping`, `search_journals` et `export_journals` sont génériques.
            if matches!(nom, "ping" | "search_journals" | "export_journals") {
                continue;
            }
            assert!(
                noms.contains(&nom),
                "is_read_only cite « {nom} », qui n'existe plus"
            );
        }

        // En miroir : rien qui écrit ne doit être classé lecture seule.
        for nom in noms {
            if nom.starts_with("delete_")
                || nom.starts_with("create_")
                || nom.starts_with("modify_")
                || nom.starts_with("adjust_")
            {
                assert!(
                    !is_read_only(nom),
                    "« {nom} » écrit mais est classé lecture seule"
                );
            }
        }
    }

    #[test]
    fn troncature_par_octets() {
        let docs = || vec![json!({"a": 1}), json!({"b": 2}), json!({"c": 3})];
        let taille = |v: &[Value]| Value::Array(v.to_vec()).to_string().len();

        // Sans limite : intact.
        assert_eq!(truncate_by_bytes(docs(), None).len(), 3);
        assert_eq!(truncate_by_bytes(docs(), Some(0)).len(), 3);

        // Borne exacte : la sortie complète passe tout juste.
        let exact = taille(&docs());
        assert_eq!(truncate_by_bytes(docs(), Some(exact)).len(), 3);
        assert_eq!(truncate_by_bytes(docs(), Some(exact - 1)).len(), 2);

        // Trop petit pour le premier document : vide, jamais tronqué au milieu.
        assert!(truncate_by_bytes(docs(), Some(3)).is_empty());

        // Invariant fort : la sortie reste du JSON valide et tient dans max.
        for max in 1..60 {
            let out = truncate_by_bytes(docs(), Some(max));
            let s = Value::Array(out).to_string();
            assert!(serde_json::from_str::<Vec<Value>>(&s).is_ok());
            assert!(s.len() <= max || s == "[]", "max={max} → {s}");
        }
    }

    #[test]
    fn formule_de_table_refusee() {
        // « 1d0 » est acceptée par Foundry à la création et faisait paniquer
        // le tirage sur `% 0` — c'est une donnée du MONDE, jamais de l'appelant.
        assert!(roll_table_draws(&table("1d0"), 0, 1).is_err());
        assert!(roll_table_draws(&table("0d6"), 0, 1).is_err());
        // Non borné, celui-ci bloquait un worker tokio pour toujours.
        assert!(roll_table_draws(&table("999999999d6"), 0, 1).is_err());
        // Le message doit citer la formule : apply_critical_injury choisit sa
        // table par NOM, donc le MJ n'a pas d'autre indice pour la retrouver.
        let err = roll_table_draws(&table("1d0"), 0, 1)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("1d0"),
            "message sans la formule fautive : {err}"
        );
    }

    #[test]
    fn formule_de_table_acceptee() {
        assert!(roll_table_draws(&table("1d100+5"), 0, 1).is_ok());
        assert!(roll_table_draws(&table("2d6-1"), 0, 1).is_ok());
        assert!(roll_table_draws(&table("1d3"), 0, 1).is_ok());
    }

    #[test]
    fn tirage_deterministe_en_1d1() {
        // 1d1 rend toujours 1 : ça épingle la sémantique `k + modifier` et la
        // sélection par plage, qui n'étaient couvertes par aucun test.
        let draws = roll_table_draws(&table("1d1"), 0, 1).unwrap();
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0]["text"], json!("un"));
        // Le modificateur décale le résultat dans les plages.
        let draws = roll_table_draws(&table("1d1"), 1, 1).unwrap();
        assert_eq!(draws[0]["text"], json!("deux"));
    }

    #[test]
    fn tirage_sans_flags_inchange() {
        // Une table sans Mini Rolltable ne gagne aucun champ.
        let d = &roll_table_draws(&table("1d1"), 0, 1).unwrap()[0];
        let keys: Vec<&String> = d.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 4, "champs inattendus : {keys:?}");
    }

    #[test]
    fn tirage_quantite_et_table_liee() {
        let t = json!({"formula": "1d1", "results": [{
            "range": [1, 1], "type": "document", "name": "Butin", "description": "",
            "documentUuid": "RollTable.sub",
            "flags": {"mini-rolltable": {"quantity": "3"}},
        }]});
        let d = &roll_table_draws(&t, 0, 1).unwrap()[0];
        assert_eq!(d["quantity"], json!(3));
        assert_eq!(d["quantityFormula"], json!("3"));
        assert_eq!(d["linkedTable"]["_id"], json!("sub"));
        // description vide → le nom sert de texte
        assert_eq!(d["text"], json!("Butin"));
        // quantité fixe « 1 » = défaut du module : pas de champ
        let t = json!({"formula": "1d1", "results": [{
            "range": [1, 1], "text": "x", "flags": {"mini-rolltable": {"quantity": "1"}}}]});
        assert!(
            roll_table_draws(&t, 0, 1).unwrap()[0]
                .get("quantity")
                .is_none()
        );
    }
}
