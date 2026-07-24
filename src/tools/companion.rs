//! Outils `client_*` : délèguent au module Foundry « foundry-mcp-gateway-companion »
//! (côté navigateur) les actions que le protocole socket seul ne peut pas faire
//! — exécuter des macros, vrais jets système (Dice So Nice), caméra, sons,
//! notifications, API Campaign Codex, télémétrie des clients.
//!
//! Mécanique : on émet une commande sur le canal `module.foundry-mcp-gateway-companion`
//! et on attend la réponse dans le buffer d'événements. Si aucun module n'est
//! installé/actif, la commande expire avec un message explicite.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::time::Duration;

use super::{ToolDef, carry, str_arg, text_response};
use crate::mcp::McpState;

const CHANNEL: &str = "module.foundry-mcp-gateway-companion";

/// Mode de livraison d'une commande côté compagnon : détermine QUI l'exécute
/// et QUI répond. C'est la table dans laquelle le handler JS est enregistré.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// SCENE_HANDLERS : chaque client ciblé exécute, le responder accuse une fois.
    Scene,
    /// ASK_HANDLERS : le client CIBLÉ exécute ET répond (poser une question).
    Addressed,
    /// UNIQUE_HANDLERS : seul le MJ « responder » exécute.
    Unique,
}

/// Chaque outil `client_*` et la commande compagnon qu'il délègue.
///
/// Source de vérité UNIQUE de la parité entre les deux dépôts : les points
/// d'appel utilisent `cmd_for(name)` plutôt qu'un littéral, si bien qu'ils ne
/// peuvent pas diverger de cette table ; `--dump-companion-commands` l'exporte
/// vers `docs/companion-commands.json`, que le `check-parity.sh` du compagnon
/// compare à ses handlers. Ajouter un `client_*` sans l'inscrire ici fait
/// échouer `chaque_client_a_une_commande`.
pub const COMPANION_COMMANDS: &[(&str, &str, Delivery)] = &[
    ("client_status", "ping_module", Delivery::Unique),
    ("client_run_macro", "run_macro", Delivery::Unique),
    ("client_run_script", "run_script", Delivery::Unique),
    (
        "client_roll_pool_native",
        "roll_pool_native",
        Delivery::Unique,
    ),
    ("client_pan_camera", "pan", Delivery::Scene),
    ("client_ping", "ping_at", Delivery::Scene),
    ("client_play_sound", "play_sound", Delivery::Scene),
    ("client_notify", "notify", Delivery::Scene),
    ("client_show_document", "show_document", Delivery::Scene),
    ("client_play_effect", "play_effect", Delivery::Unique),
    ("client_get_state", "get_client_state", Delivery::Unique),
    ("client_ask", "ask", Delivery::Addressed),
    ("client_roll_formula", "roll_formula", Delivery::Unique),
    ("client_select", "select", Delivery::Unique),
    ("client_target", "target", Delivery::Unique),
    ("client_fog", "fog", Delivery::Unique),
    ("client_weather", "weather", Delivery::Unique),
    ("client_weather_types", "weather_types", Delivery::Unique),
    ("client_token_fx", "token_fx", Delivery::Unique),
    (
        "client_token_fx_presets",
        "token_fx_presets",
        Delivery::Unique,
    ),
    ("client_effect_catalog", "effect_catalog", Delivery::Unique),
    ("client_get_derived", "get_derived", Delivery::Unique),
    ("client_enrich", "enrich", Delivery::Unique),
    ("client_search", "search", Delivery::Unique),
    ("client_capture", "capture", Delivery::Unique),
    ("client_babele", "babele", Delivery::Unique),
    ("client_scene_report", "scene_report", Delivery::Unique),
    ("client_cc_convert", "cc_convert", Delivery::Unique),
    (
        "client_cc_export_obsidian",
        "cc_export_obsidian",
        Delivery::Unique,
    ),
    ("client_cc_open_toc", "cc_open_toc", Delivery::Unique),
    ("client_al_open", "al_open", Delivery::Unique),
    ("client_mc_set_time", "mc_set_time", Delivery::Unique),
    ("client_mc_open", "mc_open", Delivery::Unique),
    ("client_mat_trigger", "mat_trigger", Delivery::Unique),
    ("client_seq_between", "seq_between", Delivery::Unique),
    ("client_seq_sound", "seq_sound", Delivery::Unique),
];

/// La commande compagnon derrière un outil `client_*`.
///
/// Les points d'appel passent par ici plutôt que par un littéral : c'est ce
/// qui rend la table VRAIE et pas seulement déclarée. Un nom hors table est un
/// bug de logique (l'outil ne devrait pas atteindre `call_companion`) — d'où
/// l'`expect`, que `chaque_client_a_une_commande` garantit inatteignable.
pub fn cmd_for(tool: &str) -> &'static str {
    COMPANION_COMMANDS
        .iter()
        .find(|(t, ..)| *t == tool)
        .map(|(_, c, _)| *c)
        .unwrap_or_else(|| panic!("{tool} is not in COMPANION_COMMANDS"))
}

/// Convention `targets` (commandes de scène) : "all" (défaut), "gm", "players",
/// ou un tableau d'_id d'utilisateurs.
// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
/// Construites une fois : `handles` est appelé pour chaque module à chaque
/// dispatch, et rebâtir les schémas json! à chaque fois coûtait une
/// allocation par outil pour une simple question d'appartenance.
static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    vec![
        ("client_status",
         "Check the companion module: is foundry-mcp-gateway-companion installed and active? Returns its version, the responding GM, and which optional deps (Dice So Nice, Campaign Codex, Sequencer) are available.",
         json!({"type":"object","properties":{}})),
        ("client_run_macro",
         "Run a Foundry macro by _id or name on the GM client (returns its return value). Unlocks anything scriptable that the socket API cannot do.",
         json!({"type":"object","properties":{
            "macro":{"type":"string"},"scope":{"type":"object","additionalProperties":true}},
            "required":["macro"]})),
        ("client_run_script",
         "Run arbitrary JavaScript on the GM client (async, receives game/canvas/ui). DISABLED by default — the GM must enable it in the module settings.",
         json!({"type":"object","properties":{"code":{"type":"string"}},"required":["code"]})),
        ("client_roll_pool_native",
         "starwarsffg: roll a dice pool with the REAL FFG engine (native chat card + Dice So Nice 3D on the table). Pass the pool (compute it with roll_actor_skill first if you want a sheet-derived pool).",
         json!({"type":"object","properties":{
            "pool":{"type":"object","additionalProperties":true,"description":"{ability,proficiency,difficulty,challenge,boost,setback,force}"},
            "description":{"type":"string"},"actor":{"type":"string","description":"actor _id for the chat speaker"}},
            "required":["pool"]})),
        ("client_pan_camera",
         "Pan (and zoom) the camera on the targeted clients — 'look here'. Give x/y, or a token _id to center on. targets: all (default) / gm / players / [userId].",
         json!({"type":"object","properties":{
            "x":{"type":"number"},"y":{"type":"number"},"scale":{"type":"number"},
            "token":{"type":"string"},"targets":{}}})),
        ("client_ping",
         "Ping a point on the map for the targeted clients. x/y or a token _id. targets: all (default) / gm / players / [userId].",
         json!({"type":"object","properties":{
            "x":{"type":"number"},"y":{"type":"number"},"token":{"type":"string"},"targets":{}}})),
        ("client_play_sound",
         "Play a one-shot sound on the targeted clients (a dramatic stinger). targets: all (default) / gm / players / [userId].",
         json!({"type":"object","properties":{
            "src":{"type":"string"},"volume":{"type":"number"},"loop":{"type":"boolean"},"targets":{}},
            "required":["src"]})),
        ("client_notify",
         "Show a UI notification on the targeted clients. type: info/warn/error. targets: all (default) / gm / players / [userId].",
         json!({"type":"object","properties":{
            "message":{"type":"string"},"type":{"type":"string"},"targets":{}},
            "required":["message"]})),
        ("client_show_document",
         "Open a document sheet (by uuid) on the targeted clients. targets: all (default) / gm / players / [userId].",
         json!({"type":"object","properties":{"uuid":{"type":"string"},"targets":{}},"required":["uuid"]})),
        ("client_play_effect",
         "Play a Sequencer visual effect (if the Sequencer module is active). At a token _id or x/y.",
         json!({"type":"object","properties":{
            "file":{"type":"string"},"token":{"type":"string"},
            "x":{"type":"number"},"y":{"type":"number"},"scale":{"type":"number"}},
            "required":["file"]})),
        ("client_get_state",
         "Client telemetry: active users, each one's viewed scene and assigned character, and the GM's current scene. (Live selections/targets stream via get_events as selection/target events.)",
         json!({"type":"object","properties":{}})),
    ]
}

/// Liste ses propres noms, et non `starts_with("client_")`.
///
/// L'ancien préfixe revendiquait les ~40 outils `client_*` alors que `run`
/// n'en traite que onze : ça ne fonctionnait que parce que ce module est
/// interrogé en neuvième position de la chaîne de `else if` du dispatch,
/// après cc_family, addons, table et insight. Réordonner cette chaîne — ou
/// définir un `client_*` dans un module système, interrogé plus tard —
/// cassait silencieusement des outils. Le test `chaque_outil_a_un_seul_
/// proprietaire` verrouille désormais l'invariant.
pub fn handles(name: &str) -> bool {
    DEFS.iter().any(|(n, _, _)| *n == name)
}

/// Comparaison semver simple (majeur.mineur.correctif, sans pré-version — ni
/// Foundry ni ce projet n'en utilisent).
fn version_lt(a: &str, b: &str) -> bool {
    let part = |v: &str| -> Vec<u32> { v.split('.').map(|x| x.parse().unwrap_or(0)).collect() };
    part(a) < part(b)
}

/// Les deux moitiés dérivent-elles ? On compare au mineur près : un correctif
/// d'un seul côté est normal, un mineur d'écart signale un protocole désaccordé.
fn drifted(a: &str, b: &str) -> bool {
    let minor = |v: &str| v.split('.').take(2).collect::<Vec<_>>().join(".");
    !a.is_empty() && minor(a) != minor(b)
}

/// Émet une commande sur le canal du module et attend la réponse (reply == id).
pub async fn call_companion(
    state: &McpState,
    cmd: &str,
    args: Value,
    targets: Option<Value>,
    timeout_secs: u64,
) -> Result<Value> {
    let id = uuid::Uuid::new_v4().to_string();

    // `argsRaw` : la sérialisation EXACTE de ce qui est signé.
    //
    // Le message traverse socket.io, qui le décode et le ré-encode côté
    // serveur Foundry : le navigateur ne voit jamais nos octets. serde_json
    // trie les clés, JSON.stringify garde l'ordre d'insertion, et les nombres
    // ne se formatent pas pareil — hacher un JSON re-sérialisé donnerait deux
    // empreintes différentes de part et d'autre. On transmet donc la chaîne,
    // et le compagnon exécute ce qu'il a vérifié plutôt qu'une reconstruction.
    // `args` reste présent : un compagnon 1.5 l'utilise et continue de tourner.
    let args_raw = args.to_string();
    // Sans monde connu, on signe une chaîne vide : le compagnon la comparera à
    // son propre game.world.id et refusera — mieux vaut un refus explicite
    // qu'une commande non liée à un monde.
    let world = state.foundry.world_id().await.unwrap_or_default();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let (sig, exp) = state.signer.sign(cmd, &id, &world, &args_raw, now_ms);

    let mut command = json!({
        "mcp": true, "cmd": cmd, "id": id, "args": args,
        "argsRaw": args_raw, "world": world, "exp": exp, "sig": sig,
    });
    if let Some(t) = targets {
        command["targets"] = t;
    }
    let since = state.foundry.event_seq();
    state.foundry.emit(CHANNEL, &[command]).await?;

    let id_for_pred = id.clone();
    let matched = state
        .foundry
        .wait_for_event(since, Duration::from_secs(timeout_secs), move |e| {
            e.event == CHANNEL
                && e.args
                    .first()
                    .and_then(|a| a.get("reply"))
                    .map(|r| r == &json!(id_for_pred))
                    .unwrap_or(false)
        })
        .await;

    let event = matched.ok_or_else(|| {
        anyhow!(
            "no companion module responded to '{cmd}' — is foundry-mcp-gateway-companion installed and active, with a GM browser connected?"
        )
    })?;
    let payload = event.args.into_iter().next().unwrap_or(Value::Null);
    if payload.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(payload.get("result").cloned().unwrap_or(Value::Null))
    } else {
        bail!(
            "companion error: {}",
            payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        )
    }
}

/// Résout un argument token/actor : renvoie l'_id tel quel (les outils client
/// attendent des _ids ; utiliser list_tokens/get_actor pour les obtenir).
fn passthrough_id(args: &Value, key: &str) -> Option<Value> {
    str_arg(args, key).map(Value::String)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let targets = args.get("targets").cloned();
    let result = match name {
        "client_status" => {
            let mut status = call_companion(state, cmd_for(name), json!({}), None, 8).await?;
            // Les deux moitiés partagent un numéro de version : elles parlent un
            // protocole commun, et un ajout d'un seul côté casse en silence. On
            // le DIT plutôt que de laisser l'utilisateur deviner.
            let theirs = status
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let ours = crate::mcp::SERVER_VERSION;
            if drifted(&theirs, ours) {
                let behind = if version_lt(&theirs, ours) {
                    "companion"
                } else {
                    "gateway"
                };
                status["gatewayVersion"] = json!(ours);
                status["versionDrift"] = json!(true);
                status["note"] = json!(format!(
                    "Version drift: gateway {ours}, companion {theirs}. The {behind} is behind — \
                     both halves speak one protocol and are released together, so update it."
                ));
            } else {
                status["gatewayVersion"] = json!(ours);
                status["versionDrift"] = json!(false);
            }
            status
        }
        "client_run_macro" => {
            let macro_ref = str_arg(args, "macro").ok_or_else(|| anyhow!("'macro' is required"))?;
            let scope = args.get("scope").cloned().unwrap_or(json!({}));
            call_companion(
                state,
                "run_macro",
                json!({ "macro": macro_ref, "scope": scope }),
                None,
                30,
            )
            .await?
        }
        "client_run_script" => {
            let code = str_arg(args, "code").ok_or_else(|| anyhow!("'code' is required"))?;
            call_companion(state, cmd_for(name), json!({ "code": code }), None, 30).await?
        }
        "client_roll_pool_native" => {
            let pool = args
                .get("pool")
                .cloned()
                .ok_or_else(|| anyhow!("'pool' is required"))?;
            let mut a = json!({ "pool": pool });
            if let Some(d) = str_arg(args, "description") {
                a["description"] = json!(d);
            }
            if let Some(actor) = passthrough_id(args, "actor") {
                a["actorId"] = actor;
            }
            call_companion(state, cmd_for(name), a, None, 30).await?
        }
        "client_pan_camera" => {
            let mut a = json!({});
            carry(args, &["x", "y", "scale"], &mut a);
            if let Some(t) = passthrough_id(args, "token") {
                a["tokenId"] = t;
            }
            call_companion(state, cmd_for(name), a, targets, 10).await?
        }
        "client_ping" => {
            let mut a = json!({});
            carry(args, &["x", "y"], &mut a);
            if let Some(t) = passthrough_id(args, "token") {
                a["tokenId"] = t;
            }
            call_companion(state, cmd_for(name), a, targets, 10).await?
        }
        "client_play_sound" => {
            let src = str_arg(args, "src").ok_or_else(|| anyhow!("'src' is required"))?;
            let mut a = json!({ "src": src });
            if let Some(v) = args.get("volume") {
                a["volume"] = v.clone();
            }
            if let Some(v) = args.get("loop") {
                a["loop"] = v.clone();
            }
            call_companion(state, cmd_for(name), a, targets, 10).await?
        }
        "client_notify" => {
            let message =
                str_arg(args, "message").ok_or_else(|| anyhow!("'message' is required"))?;
            let mut a = json!({ "message": message });
            if let Some(t) = str_arg(args, "type") {
                a["type"] = json!(t);
            }
            call_companion(state, cmd_for(name), a, targets, 10).await?
        }
        "client_show_document" => {
            let uuid = str_arg(args, "uuid").ok_or_else(|| anyhow!("'uuid' is required"))?;
            call_companion(state, cmd_for(name), json!({ "uuid": uuid }), targets, 10).await?
        }
        "client_play_effect" => {
            let file = str_arg(args, "file").ok_or_else(|| anyhow!("'file' is required"))?;
            let mut a =
                json!({ "file": file, "scale": args.get("scale").cloned().unwrap_or(json!(1)) });
            if let Some(t) = passthrough_id(args, "token") {
                a["atTokenId"] = t;
            }
            carry(args, &["x", "y"], &mut a);
            call_companion(state, cmd_for(name), a, None, 20).await?
        }
        "client_get_state" => call_companion(state, cmd_for(name), json!({}), None, 10).await?,
        other => bail!("Unknown tool: {other}"),
    };
    Ok(text_response(&result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_de_version() {
        assert!(!drifted("1.5.0", "1.5.0"));
        assert!(!drifted("1.5.3", "1.5.0")); // un correctif d'écart : normal
        assert!(drifted("0.5.0", "1.5.0")); // majeur : dérive
        assert!(drifted("1.4.0", "1.5.0")); // mineur : dérive
        assert!(!drifted("", "1.5.0")); // version inconnue : on n'invente pas

        assert!(version_lt("0.5.0", "1.5.0"));
        assert!(version_lt("1.4.9", "1.5.0"));
        assert!(!version_lt("1.5.0", "1.5.0"));
        assert!(!version_lt("1.10.0", "1.9.0")); // pas une comparaison de texte
    }

    /// Chaque outil `client_*` du catalogue doit avoir une entrée dans la
    /// table, et réciproquement : c'est ce qui garantit qu'aucune moitié ne
    /// prend de l'avance sur l'autre. Le nom de l'outil est côté Rust ; la
    /// commande, elle, doit exister côté compagnon — c'est check-parity.sh qui
    /// le vérifie, à partir du JSON exporté depuis cette même table.
    #[test]
    fn chaque_client_a_une_commande() {
        use std::collections::BTreeSet;
        // Les outils client_* sont répartis sur cinq modules, pas seulement
        // companion.rs — la parité doit tous les couvrir.
        let mut exposes = BTreeSet::new();
        for defs in [
            crate::tools::companion::definitions(),
            crate::tools::table::definitions(),
            crate::tools::insight::definitions(),
            crate::tools::cc_family::definitions(),
            crate::tools::addons::definitions(),
        ] {
            for (n, _, _) in defs {
                if n.starts_with("client_") {
                    exposes.insert(*n);
                }
            }
        }
        let en_table: BTreeSet<&str> = COMPANION_COMMANDS.iter().map(|(t, ..)| *t).collect();
        assert_eq!(
            exposes, en_table,
            "les outils client_* et COMPANION_COMMANDS ont divergé"
        );
        // Pas deux outils vers la même commande, ni deux fois le même nom.
        let cmds: BTreeSet<&str> = COMPANION_COMMANDS.iter().map(|(_, c, _)| *c).collect();
        assert_eq!(cmds.len(), COMPANION_COMMANDS.len(), "commande dupliquée");
    }

    /// Vérité-terrain écrite à la main, extraite des points d'appel réels : si
    /// la table est mal transcrite (client_ping → pan, par ex.), la parité par
    /// ensembles ne le verrait pas — mais ceci si. À tenir à jour avec la table.
    #[test]
    fn table_fidele_aux_points_d_appel() {
        let attendu: &[(&str, &str, Delivery)] = &[
            ("client_pan_camera", "pan", Delivery::Scene),
            ("client_ping", "ping_at", Delivery::Scene),
            ("client_play_sound", "play_sound", Delivery::Scene),
            ("client_notify", "notify", Delivery::Scene),
            ("client_show_document", "show_document", Delivery::Scene),
            ("client_ask", "ask", Delivery::Addressed),
            ("client_select", "select", Delivery::Unique),
            ("client_target", "target", Delivery::Unique),
            ("client_cc_convert", "cc_convert", Delivery::Unique),
            ("client_get_state", "get_client_state", Delivery::Unique),
        ];
        for (tool, cmd, delivery) in attendu {
            assert_eq!(cmd_for(tool), *cmd, "cmd de {tool}");
            let d = COMPANION_COMMANDS
                .iter()
                .find(|(t, ..)| t == tool)
                .map(|(_, _, d)| *d)
                .unwrap();
            assert_eq!(d, *delivery, "mode de livraison de {tool}");
        }
        // Exactement 5 scene, 1 addressed, 30 unique (vérifié des deux côtés).
        let n = |want: Delivery| {
            COMPANION_COMMANDS
                .iter()
                .filter(|(_, _, d)| *d == want)
                .count()
        };
        assert_eq!(n(Delivery::Scene), 5);
        assert_eq!(n(Delivery::Addressed), 1);
        assert_eq!(n(Delivery::Unique), 30);
    }
}
