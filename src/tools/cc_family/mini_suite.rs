//! Petits modules wgtnGM : Mini Countdown Dice (cd_*), Mini Light Switch
//! (ls_*), Mini Player (mp_*), Mini Rolltable (quantités et tables liées,
//! appliquées par `draw_from_table` via les helpers purs en bas de fichier).
//!
//! · Countdown Dice : tout l'état vit dans le setting monde
//!   `mini-countdown-dice.state` = {dice:[…]} ; son onChange re-rend l'overlay
//!   chez tous les clients, donc une écriture serveur suffit.
//! · Light Switch : allumer/éteindre = `AmbientLight.hidden` ; tags Tagger dans
//!   `flags.tagger.tags` ; décor animé dans `flags["wgtngm-light-switch"]`.
//!   Fonctionne sans le module (le champ hidden est du core).
//! · Mini Player : presets dans le setting `wgtgm-mini-player.soundboardPresets`,
//!   automatisation de scène dans `flags["wgtgm-mini-player"]` ; la lecture
//!   passe par l'API navigateur (compagnon), MJ seulement.

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::mcp::McpState;
use crate::tools::companion::{Delivery, call_companion, cmd_for};
use crate::tools::{ToolDef, carry, str_arg, text_response};

const CD_MODULE: &str = "mini-countdown-dice";
const CD_STATE_KEY: &str = "mini-countdown-dice.state";
const LS_MODULE: &str = "wgtngm-light-switch";
const MP_MODULE: &str = "wgtgm-mini-player";
const MP_PRESETS_KEY: &str = "wgtgm-mini-player.soundboardPresets";

/// Bornes du module (state.mjs : MIN_DIE_SIDES / MAX_DIE_SIDES).
const CD_MIN_SIDES: i64 = 2;
const CD_MAX_SIDES: i64 = 100;

/// Décors animés livrés avec Light Switch (constants.js) : (libellé, chemin).
const LS_GRAPHIC_PRESETS: [(&str, &str); 10] = [
    (
        "Large Fire",
        "modules/wgtngm-light-switch/media/largefire.webm",
    ),
    (
        "Small Fire",
        "modules/wgtngm-light-switch/media/smallfire.webm",
    ),
    (
        "Medium Fire",
        "modules/wgtngm-light-switch/media/medfire.webm",
    ),
    (
        "Small Flames",
        "modules/wgtngm-light-switch/media/smallflames.webm",
    ),
    (
        "Tiny Flames",
        "modules/wgtngm-light-switch/media/tinyflames.webm",
    ),
    ("Candle", "modules/wgtngm-light-switch/media/candle.webm"),
    (
        "Coals Round",
        "modules/wgtngm-light-switch/media/coals.webm",
    ),
    (
        "Flames Round",
        "modules/wgtngm-light-switch/media/flamesround.webm",
    ),
    (
        "Flames Large",
        "modules/wgtngm-light-switch/media/flameslarge.webm",
    ),
    (
        "Flames Wide",
        "modules/wgtngm-light-switch/media/flameswide.webm",
    ),
];

/// Flags de scène Mini Player (scene-combat-audio.js) : (argument, clé de flag).
const MP_SCENE_FLAGS: [(&str, &str); 6] = [
    ("tags", "scenePlayTags"),
    ("playlist", "scenePlayPlaylist"),
    ("playlist_sound", "scenePlayPlaylistSound"),
    ("preset", "scenePlayPreset"),
    ("stop_mini_player_on_start", "sceneStopMiniPlayerOnStart"),
    ("stop_all_sounds_on_start", "sceneStopAllSoundsOnStart"),
];

/// Mini Rolltable : profondeur max des tables liées (mini-rolltable.mjs).
pub const MRT_MAX_DEPTH: usize = 5;
/// Plafond d'une quantité tirée : une formule « 100d100 » ne doit pas faire
/// exploser la sortie (ni la récursion, où elle multiplie les tirages).
pub const MRT_MAX_QUANTITY: i64 = 100;

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

/// Outils `client_*` de ce module → commande compagnon (fusionnés dans
/// `companion::COMPANION_COMMANDS`).
pub const COMMANDS: &[(&str, &str, Delivery)] = &[
    ("client_mp_play_preset", "mp_play_preset", Delivery::Unique),
    ("client_mp_play_tags", "mp_play_tags", Delivery::Unique),
];

/// Outils sans effet de bord (annotations + mode FOUNDRY_READONLY).
pub const READ_ONLY: &[&str] = &["cd_list", "ls_lights", "mp_list_presets"];

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    let scene_props = json!({
        "scene_id":{"type":"string","description":"Scene _id (default: active scene)"},
        "scene_name":{"type":"string","description":"Scene name (default: active scene)"}});
    let mut ls_lights_props = scene_props.clone();
    ls_lights_props["tag"] = json!({"type":"string","description":"Only lights carrying this Tagger tag"});
    let mut ls_toggle_props = scene_props.clone();
    for (k, v) in [
        ("ids", json!({"type":"array","items":{"type":"string"},"description":"AmbientLight _ids"})),
        ("tags", json!({"type":"array","items":{"type":"string"},"description":"Tagger tags (a light matches if it has ANY of them)"})),
        ("tag_contains", json!({"type":"string","description":"Case-insensitive substring of a Tagger tag (AmbientLights have no name: tags are their only label)"})),
        ("all", json!({"type":"boolean","description":"Target every light of the scene"})),
        ("state", json!({"type":"string","enum":["on","off","toggle"],"description":"on = visible (hidden:false), off = hidden, toggle = flip each light"})),
        ("graphic", json!({"description":"Light Switch animated graphic: \"none\" disables it, or an object {preset (label, e.g. \"Candle\") | path, scale 0.1-10, rotation 0-360, speed 0.75|1|2}",
                           "oneOf":[{"type":"string"},{"type":"object"}]})),
    ] {
        ls_toggle_props[k] = v;
    }
    let mut mp_scene_props = scene_props.clone();
    for (k, v) in [
        ("tags", json!({"description":"Mini Player tags to play on scene activation (array or comma list; lowercased)","oneOf":[{"type":"array","items":{"type":"string"}},{"type":"string"}]})),
        ("playlist", json!({"type":"string","description":"Playlist _id or name to play on activation"})),
        ("playlist_sound", json!({"type":"string","description":"Sound _id or name inside that playlist (requires playlist)"})),
        ("preset", json!({"type":"string","description":"Soundboard preset name to apply on activation"})),
        ("stop_mini_player_on_start", json!({"type":"boolean","description":"Stop the Mini Player before applying (module default: true)"})),
        ("stop_all_sounds_on_start", json!({"type":"boolean","description":"Stop every playing sound before applying"})),
        ("clear", json!({"description":"true removes all Mini Player scene flags; or an array of argument names to remove (e.g. [\"preset\",\"tags\"])",
                          "oneOf":[{"type":"boolean"},{"type":"array","items":{"type":"string"}}]})),
    ] {
        mp_scene_props[k] = v;
    }
    vec![
        // --- Mini Countdown Dice ---
        ("cd_list",
         "Mini Countdown Dice: list the countdown dice (id, label, sides, current value, visibility, locked scene, auto-countdown config) from the world setting, plus the module options that drive them (autoCounters must be ON for autoEnabled dice to tick on turn/round).",
         json!({"type":"object","properties":{}})),
        ("cd_set",
         "Mini Countdown Dice: change the countdown dice (world setting `mini-countdown-dice.state`; every client re-renders its overlay on change). action: create (sides 2-100 required; value defaults to sides, or 1 with start=\"low\") | set_value | delta (+/-N, clamped to [1, sides]) | configure (sides, label, show_label, color #rrggbb or null, large, plain_token, background_image, auto_enabled, auto_direction increase|decrease, auto_trigger turn|round) | show | hide | lock_scene (scene_id, default active scene: only shown on that scene) | unlock_scene | remove | clear (removes ALL dice). Identify a die with `die` = its id or its label (case-insensitive, must be unique). Values are clamped to [1, sides]; auto countdown only runs if the module option autoCounters is enabled (a warning is returned otherwise).",
         json!({"type":"object","properties":{
            "action":{"type":"string","enum":["create","set_value","delta","configure","show","hide","lock_scene","unlock_scene","remove","clear"]},
            "die":{"type":"string","description":"Die id or label (all actions except create/clear)"},
            "sides":{"type":"integer","minimum":2,"maximum":100},
            "value":{"type":"integer"},"delta":{"type":"integer"},
            "start":{"type":"string","enum":["high","low"],"description":"create: start at max (default) or at 1"},
            "label":{"type":"string"},"show_label":{"type":"boolean"},
            "color":{"type":["string","null"],"description":"#rrggbb, or null for the module default colour"},
            "large":{"type":["boolean","null"]},"plain_token":{"type":["boolean","null"]},
            "background_image":{"type":"string"},
            "visible":{"type":"boolean","description":"create/configure: shown to players (default true)"},
            "position":{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"description":"Screen fraction 0-1 (default near centre)"},
            "auto_enabled":{"type":"boolean"},
            "auto_direction":{"type":"string","enum":["increase","decrease"]},
            "auto_trigger":{"type":"string","enum":["turn","round"]},
            "scene_id":{"type":"string","description":"create/configure/lock_scene: lock the die to this scene"}},
            "required":["action"]})),
        // --- Mini Light Switch ---
        ("ls_lights",
         "Mini Light Switch / core: list the AmbientLights of a scene (default: active scene) with on/off state (off = hidden), Tagger tags (flags.tagger.tags), position, colour, radii, animation and the Light Switch animated graphic (flags[\"wgtngm-light-switch\"]). Also returns the tag inventory and the graphic presets.",
         json!({"type":"object","properties":ls_lights_props})),
        ("ls_toggle",
         "Mini Light Switch / core: switch lights on/off (AmbientLight.hidden — works even without the module) and/or set their Light Switch animated graphic. Select lights with ids, tags (any match), tag_contains, or all:true (at least one selector is required). state: on | off | toggle. graphic: \"none\" or {preset|path, scale, rotation, speed}. Returns the lights changed with their before/after state.",
         json!({"type":"object","properties":ls_toggle_props})),
        // --- Mini Player ---
        ("mp_list_presets",
         "Mini Player: list the soundboard presets (world setting soundboardPresets: name and tracks with playlist/sound/path/channel) and the tags found in the legacy trackTags setting. Since Mini Player 14 track tags live in a file store: call client_mp_play_tags without tags to get the live tag list from the GM client.",
         json!({"type":"object","properties":{}})),
        ("client_mp_play_preset",
         "Mini Player (companion): apply a saved soundboard preset on the GM client (stops the current mix, then plays the preset's tracks). play_music=false keeps the music channel untouched. Requires a connected GM with Mini Player active; fails if the preset does not exist (lists the available ones).",
         json!({"type":"object","properties":{
            "name":{"type":"string"},"play_music":{"type":"boolean","description":"Default true"}},
            "required":["name"]})),
        ("client_mp_play_tags",
         "Mini Player (companion): open the Mini Player (target \"player\", default) filtered on tags (AND match; prefix a tag with \"-\" to exclude) and autoplay a matching track (mode current | random), or open the Soundboard (target \"soundboard\") on those tags. Without tags, only returns the live list of known tags. Requires a connected GM with Mini Player active.",
         json!({"type":"object","properties":{
            "tags":{"type":"array","items":{"type":"string"}},
            "target":{"type":"string","enum":["player","soundboard"]},
            "autoplay":{"type":"boolean","description":"player only, default true"},
            "mode":{"type":"string","enum":["current","random"],"description":"player autoplay mode (default current)"}}})),
        ("mp_scene_audio",
         "Mini Player: read or set the scene-activation audio automation of a scene (default: active scene) — flags[\"wgtgm-mini-player\"]: tags, playlist (+ playlist_sound), preset, stop_mini_player_on_start, stop_all_sounds_on_start. Playlist and sound names are resolved to ids. Without any setting argument, only reads. clear removes flags. The automation runs when the scene is activated by a GM with Mini Player active.",
         json!({"type":"object","properties":mp_scene_props})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    match name {
        "cd_list" => cd_list(state).await,
        "cd_set" => cd_set(state, args).await,
        "ls_lights" => ls_lights(state, args).await,
        "ls_toggle" => ls_toggle(state, args).await,
        "mp_list_presets" => mp_list_presets(state).await,
        "mp_scene_audio" => mp_scene_audio(state, args).await,
        "client_mp_play_preset" => {
            let n = str_arg(args, "name").ok_or_else(|| anyhow!("'name' is required"))?;
            let mut a = json!({"name": n});
            carry(args, &["play_music"], &mut a);
            let r = call_companion(state, cmd_for(name), a, None, 20).await?;
            Ok(text_response(&r))
        }
        "client_mp_play_tags" => {
            let mut a = json!({});
            carry(args, &["tags", "target", "autoplay", "mode"], &mut a);
            let r = call_companion(state, cmd_for(name), a, None, 20).await?;
            Ok(text_response(&r))
        }
        other => bail!("Unknown tool: {other}"),
    }
}

// ------------------------------------------------------------------ communs

/// Document Setting brut (valeur = chaîne JSON) ou None s'il n'existe pas.
async fn setting_doc(state: &McpState, key: &str) -> Result<Option<Value>> {
    let w = json!({"key": key}).as_object().cloned().unwrap();
    Ok(state
        .foundry
        .get_documents("settings", Some(&w), None, 0, Some(1))
        .await?
        .into_iter()
        .next())
}

/// Valeur décodée d'un setting (None si absent ou illisible).
fn setting_value(doc: Option<&Value>) -> Option<Value> {
    let raw = doc?.get("value")?;
    match raw {
        Value::String(s) => serde_json::from_str(s).ok(),
        other => Some(other.clone()),
    }
}

/// Upsert d'un setting monde (même forme que set_setting : valeur sérialisée).
async fn write_setting(
    state: &McpState,
    key: &str,
    existing: Option<&Value>,
    value: &Value,
) -> Result<Value> {
    let serialized = value.to_string();
    match existing {
        Some(doc) => {
            state
                .foundry
                .modify_document(
                    "Setting",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{"_id": doc["_id"], "value": serialized}],
                    }),
                )
                .await
        }
        None => {
            state
                .foundry
                .modify_document(
                    "Setting",
                    "create",
                    json!({
                        "action": "create", "renderSheet": false, "keepId": false,
                        "data": [{"key": key, "value": serialized}],
                    }),
                )
                .await
        }
    }
}

/// Module activé dans core.moduleConfiguration ?
async fn module_active(state: &McpState, id: &str) -> Result<bool> {
    let doc = setting_doc(state, "core.moduleConfiguration").await?;
    Ok(setting_value(doc.as_ref())
        .and_then(|v| v.get(id).and_then(Value::as_bool))
        .unwrap_or(false))
}

async fn require_module(state: &McpState, id: &str, label: &str) -> Result<()> {
    if !module_active(state, id).await? {
        bail!("{label} ({id}) is not active in this world");
    }
    Ok(())
}

/// Scène visée : scene_id / scene_name, sinon la scène active.
async fn resolve_scene(state: &McpState, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    let scene = match (str_arg(args, "scene_id"), str_arg(args, "scene_name")) {
        (Some(id), _) => {
            foundry
                .get_document("scenes", Some(&id), None, None)
                .await?
        }
        (None, Some(n)) => foundry.get_document("scenes", None, Some(&n), None).await?,
        _ => {
            let w = json!({"active": true}).as_object().cloned().unwrap();
            foundry
                .get_documents("scenes", Some(&w), None, 0, Some(1))
                .await?
                .into_iter()
                .next()
        }
    };
    scene.ok_or_else(|| anyhow!("Scene not found (no active scene and none specified)"))
}

async fn active_scene_id(state: &McpState) -> Result<Option<String>> {
    let w = json!({"active": true}).as_object().cloned().unwrap();
    let f = vec!["_id".to_string()];
    Ok(state
        .foundry
        .get_documents("scenes", Some(&w), Some(&f), 0, Some(1))
        .await?
        .first()
        .and_then(|s| s["_id"].as_str().map(String::from)))
}

/// Identifiant façon foundry.utils.randomID (16 caractères alphanumériques).
fn random_id() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    (0..16)
        .map(|_| CHARS[(rand::random::<f64>() * CHARS.len() as f64) as usize % CHARS.len()] as char)
        .collect()
}

// ------------------------------------------------------ Mini Countdown Dice

async fn cd_list(state: &McpState) -> Result<Value> {
    let active = module_active(state, CD_MODULE).await?;
    let doc = setting_doc(state, CD_STATE_KEY).await?;
    let dice = cd_dice(setting_value(doc.as_ref()).as_ref());
    let opt = |key: &'static str| async move {
        let d = setting_doc(state, &format!("{CD_MODULE}.{key}")).await?;
        Ok::<_, anyhow::Error>(setting_value(d.as_ref()))
    };
    let auto = opt("autoCounters").await?.and_then(|v| v.as_bool());
    let scene = active_scene_id(state).await?;
    let listed: Vec<Value> = dice
        .iter()
        .map(|d| {
            let mut d = d.clone();
            let locked = d["sceneId"].as_str().map(String::from);
            d["shownOnActiveScene"] = json!(locked.is_none() || locked == scene);
            d
        })
        .collect();
    Ok(text_response(&json!({
        "moduleActive": active,
        "count": listed.len(),
        "dice": listed,
        "options": {
            "autoCounters": auto.unwrap_or(false),
            "startMode": opt("startMode").await?.unwrap_or(json!("high")),
            "dieColor": opt("dieColor").await?.unwrap_or(json!("#8f0000")),
            "largeDice": opt("largeDice").await?.unwrap_or(json!(false)),
            "startHidden": opt("startHidden").await?.unwrap_or(json!(false)),
        },
        "activeSceneId": scene,
    })))
}

async fn cd_set(state: &McpState, args: &Value) -> Result<Value> {
    require_module(state, CD_MODULE, "Mini Countdown Dice").await?;
    let action = str_arg(args, "action").ok_or_else(|| anyhow!("'action' is required"))?;
    let doc = setting_doc(state, CD_STATE_KEY).await?;
    let mut dice = cd_dice(setting_value(doc.as_ref()).as_ref());

    // Valeurs par défaut lues dans le monde, seulement quand elles servent.
    let mut ctx = CdContext::default();
    if action == "create" {
        let start = setting_value(
            setting_doc(state, &format!("{CD_MODULE}.startMode"))
                .await?
                .as_ref(),
        );
        let hidden = setting_value(
            setting_doc(state, &format!("{CD_MODULE}.startHidden"))
                .await?
                .as_ref(),
        );
        ctx.start_low = start.as_ref().and_then(Value::as_str) == Some("low");
        ctx.start_hidden = hidden.and_then(|v| v.as_bool()).unwrap_or(false);
        ctx.new_id = random_id();
    }
    if action == "lock_scene" && str_arg(args, "scene_id").is_none() {
        ctx.active_scene = active_scene_id(state).await?;
    }

    let outcome = cd_apply(&mut dice, &action, args, &ctx)?;
    let result = write_setting(state, CD_STATE_KEY, doc.as_ref(), &json!({"dice": dice})).await?;

    let mut warnings = Vec::new();
    if dice.iter().any(|d| d["autoEnabled"] == json!(true)) {
        let auto = setting_value(
            setting_doc(state, &format!("{CD_MODULE}.autoCounters"))
                .await?
                .as_ref(),
        );
        if !auto.and_then(|v| v.as_bool()).unwrap_or(false) {
            warnings.push(
                "autoEnabled dice will NOT tick: the module option 'Automatic counters' (mini-countdown-dice.autoCounters) is off",
            );
        }
    }
    Ok(text_response(&json!({
        "action": action,
        "die": outcome,
        "count": dice.len(),
        "warnings": warnings,
        "ok": result.get("error").is_none_or(Value::is_null),
    })))
}

/// Paramètres de `cd_apply` qui viennent du monde (et non de l'appelant).
#[derive(Default)]
struct CdContext {
    start_low: bool,
    start_hidden: bool,
    new_id: String,
    active_scene: Option<String>,
}

/// Dés du setting, normalisés comme `normalizeState` (state.mjs) : un dé à
/// taille invalide disparaît, la valeur est bornée, les champs manquants
/// reçoivent leur défaut.
fn cd_dice(state: Option<&Value>) -> Vec<Value> {
    let Some(arr) = state.and_then(|s| s.get("dice")).and_then(Value::as_array) else {
        return vec![];
    };
    arr.iter()
        .filter_map(|d| {
            let sides = d.get("sides").and_then(Value::as_i64)?;
            if !(CD_MIN_SIDES..=CD_MAX_SIDES).contains(&sides) {
                return None;
            }
            let mut out = d.clone();
            let value = d
                .get("value")
                .and_then(Value::as_f64)
                .unwrap_or(1.0)
                .round() as i64;
            out["value"] = json!(value.clamp(1, sides));
            for (k, def) in [
                ("visible", json!(true)),
                ("position", json!({"x": 0.5, "y": 0.5})),
                ("color", Value::Null),
                ("large", Value::Null),
                ("sceneId", Value::Null),
                ("label", json!("")),
                ("showLabel", json!(false)),
                ("plainToken", Value::Null),
                ("backgroundImage", json!("")),
                ("autoEnabled", json!(false)),
                ("autoDirection", json!("decrease")),
                ("autoTrigger", json!("turn")),
            ] {
                if out.get(k).is_none() {
                    out[k] = def;
                }
            }
            Some(out)
        })
        .collect()
}

fn cd_sides(v: &Value) -> Result<i64> {
    let s = v
        .as_i64()
        .ok_or_else(|| anyhow!("'sides' must be an integer"))?;
    if !(CD_MIN_SIDES..=CD_MAX_SIDES).contains(&s) {
        bail!("'sides' must be between {CD_MIN_SIDES} and {CD_MAX_SIDES} (got {s})");
    }
    Ok(s)
}

/// Couleur #rrggbb (minuscules, comme normalizeDieColor) ou null = défaut.
fn cd_color(v: &Value) -> Result<Value> {
    match v {
        Value::Null => Ok(Value::Null),
        Value::String(s) if s.trim().is_empty() => Ok(Value::Null),
        Value::String(s) => {
            let c = s.trim().to_lowercase();
            let ok = c.len() == 7
                && c.starts_with('#')
                && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
            if !ok {
                bail!("'color' must be #rrggbb (got {s})");
            }
            Ok(json!(c))
        }
        _ => bail!("'color' must be a string #rrggbb or null"),
    }
}

fn cd_enum(args: &Value, key: &str, allowed: &[&str]) -> Result<Option<Value>> {
    match args.get(key) {
        None => Ok(None),
        Some(v) => {
            let s = v.as_str().unwrap_or_default();
            if !allowed.contains(&s) {
                bail!("'{key}' must be one of {} (got {v})", allowed.join(" | "));
            }
            Ok(Some(json!(s)))
        }
    }
}

/// Retrouve un dé par id, sinon par libellé (insensible à la casse, unique).
fn cd_find(dice: &[Value], ident: &str) -> Result<usize> {
    if let Some(i) = dice.iter().position(|d| d["id"].as_str() == Some(ident)) {
        return Ok(i);
    }
    let wanted = ident.trim().to_lowercase();
    let hits: Vec<usize> = dice
        .iter()
        .enumerate()
        .filter(|(_, d)| {
            d["label"].as_str().map(|l| l.trim().to_lowercase()) == Some(wanted.clone())
        })
        .map(|(i, _)| i)
        .collect();
    match hits.as_slice() {
        [i] => Ok(*i),
        [] => bail!(
            "No countdown die with id or label '{ident}' (known: {})",
            dice.iter()
                .map(|d| format!(
                    "{} \"{}\"",
                    d["id"].as_str().unwrap_or("?"),
                    d["label"].as_str().unwrap_or("")
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => bail!("Several dice are labelled '{ident}': use the die id"),
    }
}

/// Applique les champs de configuration fournis (create et configure).
fn cd_configure(die: &mut Value, args: &Value) -> Result<()> {
    if let Some(s) = args.get("sides") {
        let s = cd_sides(s)?;
        die["sides"] = json!(s);
        let v = die["value"].as_i64().unwrap_or(s).clamp(1, s);
        die["value"] = json!(v);
    }
    if let Some(c) = args.get("color") {
        die["color"] = cd_color(c)?;
    }
    if let Some(l) = args.get("label") {
        die["label"] = json!(l.as_str().unwrap_or_default());
    }
    for (arg, field) in [
        ("show_label", "showLabel"),
        ("visible", "visible"),
        ("auto_enabled", "autoEnabled"),
    ] {
        if let Some(v) = args.get(arg) {
            die[field] = json!(
                v.as_bool()
                    .ok_or_else(|| anyhow!("'{arg}' must be a boolean"))?
            );
        }
    }
    for (arg, field) in [("large", "large"), ("plain_token", "plainToken")] {
        if let Some(v) = args.get(arg) {
            die[field] = if v.is_boolean() {
                v.clone()
            } else {
                Value::Null
            };
        }
    }
    if let Some(b) = args.get("background_image") {
        die["backgroundImage"] = json!(b.as_str().unwrap_or_default().trim());
    }
    if let Some(d) = cd_enum(args, "auto_direction", &["increase", "decrease"])? {
        die["autoDirection"] = d;
    }
    if let Some(t) = cd_enum(args, "auto_trigger", &["turn", "round"])? {
        die["autoTrigger"] = t;
    }
    if let Some(p) = args.get("position") {
        let c = |k: &str| p.get(k).and_then(Value::as_f64).map(|v| v.clamp(0.0, 1.0));
        let (x, y) = (c("x"), c("y"));
        die["position"] = json!({
            "x": x.or(die.pointer("/position/x").and_then(Value::as_f64)).unwrap_or(0.5),
            "y": y.or(die.pointer("/position/y").and_then(Value::as_f64)).unwrap_or(0.5),
        });
    }
    if let Some(s) = args.get("scene_id") {
        die["sceneId"] = s
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map_or(Value::Null, |s| json!(s.trim()));
    }
    if let Some(v) = args.get("value") {
        let sides = die["sides"].as_i64().unwrap_or(CD_MAX_SIDES);
        let v = v
            .as_i64()
            .ok_or_else(|| anyhow!("'value' must be an integer"))?;
        die["value"] = json!(v.clamp(1, sides));
    }
    Ok(())
}

/// Cœur pur de cd_set : modifie `dice` et rend le dé touché (ou un résumé).
fn cd_apply(dice: &mut Vec<Value>, action: &str, args: &Value, ctx: &CdContext) -> Result<Value> {
    let ident = || {
        str_arg(args, "die")
            .ok_or_else(|| anyhow!("'die' (id or label) is required for action {action}"))
    };
    match action {
        "create" => {
            let sides = cd_sides(
                args.get("sides")
                    .ok_or_else(|| anyhow!("'sides' is required to create a die"))?,
            )?;
            let start_low = match str_arg(args, "start").as_deref() {
                Some("low") => true,
                Some("high") => false,
                Some(o) => bail!("'start' must be high or low (got {o})"),
                None => ctx.start_low,
            };
            let jitter = || 0.5 + (rand::random::<f64>() - 0.5) * 0.2;
            let label = str_arg(args, "label").unwrap_or_default();
            let mut die = json!({
                "id": if ctx.new_id.is_empty() { random_id() } else { ctx.new_id.clone() },
                "sides": sides,
                "value": if start_low { 1 } else { sides },
                "visible": !ctx.start_hidden,
                "position": {"x": jitter(), "y": jitter()},
                "color": null, "large": null, "sceneId": null,
                "label": label, "showLabel": !label.is_empty(),
                "plainToken": null, "backgroundImage": "",
                "autoEnabled": false, "autoDirection": "decrease", "autoTrigger": "turn",
            });
            let mut rest = args.clone();
            if let Some(o) = rest.as_object_mut() {
                o.remove("sides");
            }
            cd_configure(&mut die, &rest)?;
            dice.push(die.clone());
            Ok(die)
        }
        "clear" => {
            let n = dice.len();
            dice.clear();
            Ok(json!({"removed": n}))
        }
        _ => {
            let i = cd_find(dice, &ident()?)?;
            let die = &mut dice[i];
            let sides = die["sides"].as_i64().unwrap_or(CD_MAX_SIDES);
            let current = die["value"].as_i64().unwrap_or(1);
            match action {
                "set_value" => {
                    let v = args
                        .get("value")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| anyhow!("'value' (integer) is required"))?;
                    if !(1..=sides).contains(&v) {
                        bail!("'value' must be between 1 and {sides} for this d{sides} (got {v})");
                    }
                    die["value"] = json!(v);
                }
                "delta" => {
                    let d = args
                        .get("delta")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| anyhow!("'delta' (integer) is required"))?;
                    die["value"] = json!((current + d).clamp(1, sides));
                }
                "configure" => cd_configure(die, args)?,
                "show" => die["visible"] = json!(true),
                "hide" => die["visible"] = json!(false),
                "lock_scene" => {
                    let s = str_arg(args, "scene_id")
                        .or_else(|| ctx.active_scene.clone())
                        .ok_or_else(|| anyhow!("No active scene: give scene_id"))?;
                    die["sceneId"] = json!(s);
                }
                "unlock_scene" => die["sceneId"] = Value::Null,
                "remove" => {
                    let removed = dice.remove(i);
                    return Ok(json!({"removed": removed}));
                }
                other => bail!("Unknown action '{other}'"),
            }
            let mut out = die.clone();
            out["previousValue"] = json!(current);
            Ok(out)
        }
    }
}

// ------------------------------------------------------- Mini Light Switch

/// Tags Tagger d'un document (tableau, chaîne « a, b » ou valeur seule).
fn tagger_tags(doc: &Value) -> Vec<String> {
    let raw = doc.pointer("/flags/tagger/tags");
    let vals: Vec<String> = match raw {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect(),
        Some(Value::String(s)) => s.split(',').map(String::from).collect(),
        _ => vec![],
    };
    let mut out: Vec<String> = Vec::new();
    for t in vals
        .iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
    {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

fn ls_graphic(light: &Value) -> Value {
    let f = light.pointer(&format!("/flags/{LS_MODULE}"));
    let get = |k: &str| f.and_then(|f| f.get(k)).cloned().unwrap_or(Value::Null);
    let path = get("lightGraphicPath");
    let preset = path.as_str().and_then(|p| {
        LS_GRAPHIC_PRESETS
            .iter()
            .find(|(_, pp)| *pp == p)
            .map(|(l, _)| *l)
    });
    json!({
        "enabled": get("lightGraphicEnabled").as_bool().unwrap_or(false),
        "path": path, "preset": preset,
        "scale": get("lightGraphicScale"), "rotation": get("lightGraphicRotation"),
        "speed": get("lightGraphicSpeed"),
    })
}

fn ls_summary(light: &Value) -> Value {
    let hidden = light["hidden"].as_bool().unwrap_or(false);
    json!({
        "_id": light["_id"],
        "state": if hidden { "off" } else { "on" },
        "hidden": hidden,
        "tags": tagger_tags(light),
        "x": light["x"], "y": light["y"],
        "color": light.pointer("/config/color"),
        "bright": light.pointer("/config/bright"), "dim": light.pointer("/config/dim"),
        "animation": light.pointer("/config/animation/type"),
        "graphic": ls_graphic(light),
    })
}

/// Sélecteur de lumières de ls_toggle (pur).
struct LightSelector {
    all: bool,
    ids: Vec<String>,
    tags: Vec<String>,
    tag_contains: Option<String>,
}

impl LightSelector {
    fn from_args(args: &Value) -> Result<Self> {
        let list = |k: &str| -> Vec<String> {
            args.get(k)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let s = Self {
            all: args.get("all").and_then(Value::as_bool).unwrap_or(false),
            ids: list("ids"),
            tags: list("tags"),
            tag_contains: str_arg(args, "tag_contains").filter(|s| !s.trim().is_empty()),
        };
        if !s.all && s.ids.is_empty() && s.tags.is_empty() && s.tag_contains.is_none() {
            bail!("Select lights with ids, tags, tag_contains or all:true");
        }
        Ok(s)
    }

    fn matches(&self, light: &Value) -> bool {
        if self.all {
            return true;
        }
        let id = light["_id"].as_str().unwrap_or_default();
        let tags = tagger_tags(light);
        self.ids.iter().any(|i| i == id)
            || self
                .tags
                .iter()
                .any(|t| tags.iter().any(|lt| lt.eq_ignore_ascii_case(t)))
            || self.tag_contains.as_ref().is_some_and(|needle| {
                let n = needle.to_lowercase();
                tags.iter().any(|lt| lt.to_lowercase().contains(&n))
            })
    }
}

/// Flags Light Switch à écrire pour `graphic` (None si absent).
fn ls_graphic_flags(graphic: Option<&Value>) -> Result<Option<Map<String, Value>>> {
    let Some(g) = graphic else { return Ok(None) };
    let mut f = Map::new();
    match g {
        Value::String(s) if matches!(s.as_str(), "none" | "off" | "") => {
            f.insert("lightGraphicEnabled".into(), json!(false));
        }
        Value::String(s) => return ls_graphic_flags(Some(&json!({"preset": s}))),
        Value::Object(o) => {
            let path = match (
                o.get("preset").and_then(Value::as_str),
                o.get("path").and_then(Value::as_str),
            ) {
                (Some(p), _) => LS_GRAPHIC_PRESETS
                    .iter()
                    .find(|(l, _)| l.eq_ignore_ascii_case(p.trim()))
                    .map(|(_, path)| path.to_string())
                    .ok_or_else(|| {
                        anyhow!(
                            "Unknown Light Switch graphic preset '{p}' (known: {})",
                            LS_GRAPHIC_PRESETS
                                .iter()
                                .map(|(l, _)| *l)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?,
                (None, Some(p)) if !p.trim().is_empty() => p.trim().to_string(),
                _ => bail!("graphic needs a preset or a path (or \"none\")"),
            };
            let enabled = o.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            f.insert("lightGraphicEnabled".into(), json!(enabled));
            f.insert("lightGraphicPath".into(), json!(path));
            if let Some(s) = o.get("scale") {
                let s = s
                    .as_f64()
                    .filter(|s| (0.1..=10.0).contains(s))
                    .ok_or_else(|| anyhow!("graphic.scale must be between 0.1 and 10"))?;
                f.insert("lightGraphicScale".into(), json!(s));
            }
            if let Some(r) = o.get("rotation") {
                let r = r
                    .as_f64()
                    .filter(|r| (0.0..=360.0).contains(r))
                    .ok_or_else(|| anyhow!("graphic.rotation must be between 0 and 360"))?;
                f.insert("lightGraphicRotation".into(), json!(r));
            }
            if let Some(s) = o.get("speed") {
                let s = s
                    .as_f64()
                    .filter(|s| [0.75, 1.0, 2.0].contains(s))
                    .ok_or_else(|| anyhow!("graphic.speed must be 0.75, 1 or 2"))?;
                f.insert("lightGraphicSpeed".into(), json!(s));
            }
        }
        _ => bail!("graphic must be \"none\" or an object {{preset|path, scale, rotation, speed}}"),
    }
    Ok(Some(f))
}

async fn ls_lights(state: &McpState, args: &Value) -> Result<Value> {
    let scene = resolve_scene(state, args).await?;
    let empty = vec![];
    let lights = scene
        .get("lights")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let filter = str_arg(args, "tag");
    let listed: Vec<Value> = lights
        .iter()
        .filter(|l| {
            filter
                .as_ref()
                .is_none_or(|t| tagger_tags(l).iter().any(|lt| lt.eq_ignore_ascii_case(t)))
        })
        .map(ls_summary)
        .collect();
    let mut inventory: Vec<(String, usize)> = Vec::new();
    for l in lights {
        for t in tagger_tags(l) {
            match inventory.iter_mut().find(|(k, _)| *k == t) {
                Some(e) => e.1 += 1,
                None => inventory.push((t, 1)),
            }
        }
    }
    let on = listed.iter().filter(|l| l["state"] == "on").count();
    Ok(text_response(&json!({
        "scene": {"_id": scene["_id"], "name": scene["name"], "active": scene["active"]},
        "lightSwitchActive": module_active(state, LS_MODULE).await?,
        "taggerActive": module_active(state, "tagger").await?,
        "count": listed.len(), "on": on, "off": listed.len() - on,
        "lights": listed,
        "tags": inventory.into_iter().map(|(t, n)| json!({"tag": t, "lights": n})).collect::<Vec<_>>(),
        "graphicPresets": LS_GRAPHIC_PRESETS.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
    })))
}

async fn ls_toggle(state: &McpState, args: &Value) -> Result<Value> {
    let selector = LightSelector::from_args(args)?;
    let mode = str_arg(args, "state");
    if let Some(m) = &mode
        && !matches!(m.as_str(), "on" | "off" | "toggle")
    {
        bail!("'state' must be on, off or toggle (got {m})");
    }
    let graphic = ls_graphic_flags(args.get("graphic"))?;
    if mode.is_none() && graphic.is_none() {
        bail!("Nothing to do: give state (on|off|toggle) and/or graphic");
    }
    let scene = resolve_scene(state, args).await?;
    let empty = vec![];
    let lights = scene
        .get("lights")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let targets: Vec<&Value> = lights.iter().filter(|l| selector.matches(l)).collect();
    if targets.is_empty() {
        bail!(
            "No light matches the selection on scene '{}' ({} lights)",
            scene["name"].as_str().unwrap_or("?"),
            lights.len()
        );
    }
    let mut updates = Vec::new();
    let mut changes = Vec::new();
    for l in &targets {
        let before = l["hidden"].as_bool().unwrap_or(false);
        let after = match mode.as_deref() {
            Some("on") => false,
            Some("off") => true,
            Some(_) => !before,
            None => before,
        };
        let mut u = json!({"_id": l["_id"]});
        if after != before {
            u["hidden"] = json!(after);
        }
        if let Some(g) = &graphic {
            u["flags"] = json!({ LS_MODULE: g });
        }
        if u.as_object().is_some_and(|o| o.len() > 1) {
            updates.push(u);
        }
        changes.push(json!({
            "_id": l["_id"], "tags": tagger_tags(l),
            "before": if before { "off" } else { "on" },
            "after": if after { "off" } else { "on" },
        }));
    }
    let result = if updates.is_empty() {
        Value::Null
    } else {
        state
            .foundry
            .modify_document(
                "AmbientLight",
                "update",
                json!({
                    "action": "update", "diff": false, "recursive": true, "render": true,
                    "parentUuid": format!("Scene.{}", scene["_id"].as_str().unwrap_or("")),
                    "updates": updates,
                }),
            )
            .await?
    };
    Ok(text_response(&json!({
        "scene": {"_id": scene["_id"], "name": scene["name"]},
        "matched": targets.len(), "updated": updates.len(),
        "lights": changes, "graphic": graphic,
        "result": result,
    })))
}

// ------------------------------------------------------------- Mini Player

/// Presets du setting soundboardPresets, triés par nom (comme getPresetEntries).
fn mp_presets(store: Option<&Value>) -> Vec<Value> {
    let Some(obj) = store.and_then(Value::as_object) else {
        return vec![];
    };
    let mut out: Vec<Value> = obj
        .iter()
        .filter_map(|(key, p)| {
            let name = p.get("name").and_then(Value::as_str).unwrap_or(key).trim().to_string();
            if name.is_empty() {
                return None;
            }
            let tracks: Vec<Value> = p
                .get("tracks")
                .and_then(Value::as_array)
                .map(|ts| {
                    ts.iter()
                        .map(|t| {
                            json!({
                                "soundName": t.get("soundName"), "playlistName": t.get("playlistName"),
                                "path": t.get("path"), "channel": t.get("channel"),
                                "state": t.get("state"), "volume": t.get("volume"),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(json!({"key": key, "name": name, "trackCount": tracks.len(), "tracks": tracks}))
        })
        .collect();
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    out
}

/// Normalise une liste de tags Mini Player : minuscules, sans doublons.
fn mp_tag_list(v: &Value) -> Vec<String> {
    let raw: Vec<String> = match v {
        Value::Array(a) => a
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect(),
        Value::String(s) => s.split(',').map(String::from).collect(),
        _ => vec![],
    };
    let mut out: Vec<String> = Vec::new();
    for t in raw
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
    {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

async fn mp_list_presets(state: &McpState) -> Result<Value> {
    let active = module_active(state, MP_MODULE).await?;
    let presets =
        mp_presets(setting_value(setting_doc(state, MP_PRESETS_KEY).await?.as_ref()).as_ref());
    let legacy = setting_value(
        setting_doc(state, &format!("{MP_MODULE}.trackTags"))
            .await?
            .as_ref(),
    );
    let mut tags: Vec<String> = Vec::new();
    if let Some(map) = legacy.as_ref().and_then(Value::as_object) {
        for t in map.values().flat_map(mp_tag_list) {
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
    }
    tags.sort();
    let migrated = setting_value(
        setting_doc(state, &format!("{MP_MODULE}.trackTagsFileStoreMigrated"))
            .await?
            .as_ref(),
    )
    .and_then(|v| v.as_bool())
    .unwrap_or(false);
    Ok(text_response(&json!({
        "moduleActive": active,
        "count": presets.len(),
        "presets": presets,
        "legacyTags": tags,
        "tagsInFileStore": migrated,
        "hint": "Live track tags: call client_mp_play_tags without tags.",
    })))
}

/// Construit l'update de flags de mp_scene_audio (pur). `playlist` et
/// `playlist_sound` doivent déjà être des ids. Rend (update, clés retirées).
fn mp_scene_update(args: &Value) -> Result<(Map<String, Value>, Vec<&'static str>)> {
    let mut set = Map::new();
    let mut removed = Vec::new();
    match args.get("clear") {
        None | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => removed.extend(MP_SCENE_FLAGS.iter().map(|(_, f)| *f)),
        Some(Value::Array(a)) => {
            for k in a.iter().filter_map(Value::as_str) {
                let (_, f) = MP_SCENE_FLAGS
                    .iter()
                    .find(|(arg, f)| *arg == k || *f == k)
                    .ok_or_else(|| anyhow!("clear: unknown key '{k}'"))?;
                removed.push(*f);
            }
        }
        Some(_) => bail!("'clear' must be true or an array of keys"),
    }
    for (arg, flag) in MP_SCENE_FLAGS {
        let Some(v) = args.get(arg) else { continue };
        let value = match arg {
            "tags" => json!(mp_tag_list(v).join(", ")),
            "stop_mini_player_on_start" | "stop_all_sounds_on_start" => {
                json!(
                    v.as_bool()
                        .ok_or_else(|| anyhow!("'{arg}' must be a boolean"))?
                )
            }
            _ => json!(
                v.as_str()
                    .ok_or_else(|| anyhow!("'{arg}' must be a string"))?
                    .trim()
            ),
        };
        removed.retain(|f| *f != flag);
        set.insert(flag.to_string(), value);
    }
    let mut update = Map::new();
    for (k, v) in set {
        update.insert(format!("flags.{MP_MODULE}.{k}"), v);
    }
    for f in &removed {
        update.insert(format!("flags.{MP_MODULE}.-={f}"), Value::Null);
    }
    Ok((update, removed))
}

fn mp_scene_view(scene: &Value) -> Value {
    let f = scene.pointer(&format!("/flags/{MP_MODULE}"));
    let mut out = Map::new();
    for (arg, flag) in MP_SCENE_FLAGS {
        out.insert(
            arg.to_string(),
            f.and_then(|f| f.get(flag)).cloned().unwrap_or(Value::Null),
        );
    }
    Value::Object(out)
}

async fn mp_scene_audio(state: &McpState, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    let scene = resolve_scene(state, args).await?;
    let writes =
        MP_SCENE_FLAGS.iter().any(|(a, _)| args.get(*a).is_some()) || args.get("clear").is_some();
    let current = mp_scene_view(&scene);
    if !writes {
        return Ok(text_response(&json!({
            "scene": {"_id": scene["_id"], "name": scene["name"]},
            "moduleActive": module_active(state, MP_MODULE).await?,
            "audio": current,
        })));
    }
    require_module(state, MP_MODULE, "Mini Player").await?;
    // Playlist et son : noms → ids (le module ne lit que des ids).
    let mut a = args.clone();
    let mut resolved = json!({});
    if let Some(p) = str_arg(args, "playlist").filter(|p| !p.trim().is_empty()) {
        let pl = foundry
            .find_document("playlists", &p, None)
            .await?
            .ok_or_else(|| anyhow!("Playlist not found: {p}"))?;
        a["playlist"] = pl["_id"].clone();
        resolved["playlist"] = json!({"_id": pl["_id"], "name": pl["name"]});
        if let Some(s) = str_arg(args, "playlist_sound").filter(|s| !s.trim().is_empty()) {
            let snd = pl
                .get("sounds")
                .and_then(Value::as_array)
                .and_then(|ss| {
                    ss.iter()
                        .find(|x| x["_id"].as_str() == Some(&s))
                        .or_else(|| ss.iter().find(|x| x["name"].as_str() == Some(&s)))
                })
                .ok_or_else(|| {
                    anyhow!(
                        "Sound '{s}' not found in playlist '{}'",
                        pl["name"].as_str().unwrap_or("?")
                    )
                })?;
            a["playlist_sound"] = snd["_id"].clone();
            resolved["playlist_sound"] = json!({"_id": snd["_id"], "name": snd["name"]});
        }
    } else if args
        .get("playlist_sound")
        .is_some_and(|s| s.as_str().is_some_and(|s| !s.trim().is_empty()))
    {
        bail!("playlist_sound requires playlist");
    }
    if let Some(p) = str_arg(args, "preset").filter(|p| !p.trim().is_empty()) {
        let presets =
            mp_presets(setting_value(setting_doc(state, MP_PRESETS_KEY).await?.as_ref()).as_ref());
        let hit = presets
            .iter()
            .find(|x| {
                x["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(p.trim()))
            })
            .ok_or_else(|| {
                anyhow!(
                    "Mini Player preset '{p}' not found (known: {})",
                    presets
                        .iter()
                        .filter_map(|x| x["name"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        a["preset"] = hit["name"].clone();
    }
    let (mut update, removed) = mp_scene_update(&a)?;
    update.insert("_id".into(), scene["_id"].clone());
    let result = foundry
        .modify_document(
            "Scene",
            "update",
            json!({
                "action": "update", "diff": false, "recursive": true, "render": true,
                "updates": [Value::Object(update)],
            }),
        )
        .await?;
    let after = foundry
        .get_document(
            "scenes",
            scene["_id"].as_str(),
            None,
            Some(&["_id".into(), "flags".into()]),
        )
        .await?
        .map(|s| mp_scene_view(&s));
    Ok(text_response(&json!({
        "scene": {"_id": scene["_id"], "name": scene["name"]},
        "before": current, "audio": after, "removed": removed, "resolved": resolved,
        "ok": result.get("error").is_none_or(Value::is_null),
    })))
}

// ------------------------------------------- Mini Rolltable (draw_from_table)

/// Formule de quantité Mini Rolltable (flags["mini-rolltable"].quantity) d'un
/// résultat, si elle diffère du défaut « 1 ».
pub fn mrt_quantity_formula(result: &Value) -> Option<String> {
    let raw = result.pointer("/flags/mini-rolltable/quantity")?;
    let f = match raw {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!f.is_empty() && f != "1").then_some(f)
}

/// Tire une quantité : entier « k », « NdM », « NdM±k » (N ≤ 100, M ≤ 1000).
/// Résultat borné à [0, MRT_MAX_QUANTITY], comme le module qui plancher à 0.
/// Une formule non supportée vaut 1 (le module fait pareil sur formule
/// invalide) avec un avertissement.
pub fn mrt_roll_quantity(formula: &str, mut rng: impl FnMut() -> f64) -> (i64, Option<String>) {
    let f = formula.replace(char::is_whitespace, "");
    if let Ok(k) = f.parse::<i64>() {
        return (k.clamp(0, MRT_MAX_QUANTITY), None);
    }
    let re = regex::Regex::new(r"^(?i)(\d*)d(\d+)([+-]\d+)?$").unwrap();
    let Some(c) = re.captures(&f) else {
        return (
            1,
            Some(format!("unsupported quantity formula '{formula}', used 1")),
        );
    };
    let n: i64 = if c[1].is_empty() {
        1
    } else {
        c[1].parse().unwrap_or(0)
    };
    let m: i64 = c[2].parse().unwrap_or(0);
    if !(1..=100).contains(&n) || !(1..=1000).contains(&m) {
        return (
            1,
            Some(format!(
                "quantity formula '{formula}' out of bounds, used 1"
            )),
        );
    }
    let k: i64 = c.get(3).and_then(|k| k.as_str().parse().ok()).unwrap_or(0);
    let mut total = k;
    for _ in 0..n {
        total += 1 + ((rng() * m as f64) as i64).min(m - 1);
    }
    let clamped = total.clamp(0, MRT_MAX_QUANTITY);
    let warn =
        (clamped != total.max(0)).then(|| format!("quantity {total} capped at {MRT_MAX_QUANTITY}"));
    (clamped, warn)
}

/// Résultat lié à une autre RollTable du monde ? Rend (uuid, id du monde si
/// la table est dans le monde). Formes v13+ (type document + documentUuid) et
/// v11-12 (documentCollection + documentId).
pub fn mrt_linked_table(result: &Value) -> Option<(String, Option<String>)> {
    let ty = result.get("type");
    let is_doc = matches!(ty, Some(Value::String(s)) if s == "document" || s == "pack")
        || matches!(ty, Some(Value::Number(n)) if n.as_i64() == Some(1) || n.as_i64() == Some(2));
    if !is_doc {
        return None;
    }
    if let Some(uuid) = result.get("documentUuid").and_then(Value::as_str) {
        let parts: Vec<&str> = uuid.split('.').collect();
        return match parts.as_slice() {
            ["RollTable", id] => Some((uuid.to_string(), Some(id.to_string()))),
            [.., "RollTable", _] => Some((uuid.to_string(), None)),
            _ => None,
        };
    }
    let coll = result.get("documentCollection").and_then(Value::as_str)?;
    let id = result.get("documentId").and_then(Value::as_str)?;
    if coll == "RollTable" {
        Some((format!("RollTable.{id}"), Some(id.to_string())))
    } else if coll.contains('.') {
        // compendium : on ne sait pas si c'est une table sans la charger
        None
    } else {
        None
    }
}

/// Rendu HTML (chat) d'un tirage et de ses sous-tirages, récursif.
pub fn mrt_render_draws(draws: &[Value], modifier: i64, top: bool) -> String {
    let mut html = String::new();
    for d in draws {
        let q = d.get("quantity").and_then(Value::as_i64).unwrap_or(1);
        let qty = if d.get("quantityFormula").is_some() || q != 1 {
            format!(" <em>×{q}</em>")
        } else {
            String::new()
        };
        let text = d["text"]
            .as_str()
            .filter(|t| !t.is_empty())
            .unwrap_or("<em>hors table</em>");
        let modi = if top && modifier != 0 {
            format!(" (dont {modifier:+})")
        } else {
            String::new()
        };
        let line = format!("<strong>{}</strong>{modi} → {text}{qty}", d["roll"]);
        let sub = d
            .get("subDraws")
            .and_then(Value::as_array)
            .filter(|s| !s.is_empty())
            .map(|s| format!("<ul>{}</ul>", mrt_render_draws(s, 0, false)))
            .unwrap_or_default();
        if top {
            html.push_str(&format!("<p>{line}</p>{sub}\n"));
        } else {
            html.push_str(&format!("<li>{line}{sub}</li>"));
        }
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> CdContext {
        CdContext {
            new_id: "ID000000000000AA".into(),
            ..Default::default()
        }
    }

    #[test]
    fn cd_create_valide_et_borne() {
        let mut dice = vec![];
        let d = cd_apply(
            &mut dice,
            "create",
            &json!({"sides": 6, "label": "Alarme", "color": "#FF0000"}),
            &ctx(),
        )
        .unwrap();
        assert_eq!(d["value"], json!(6));
        assert_eq!(d["color"], json!("#ff0000"));
        assert_eq!(d["showLabel"], json!(true));
        assert_eq!(dice.len(), 1);
        // start low
        let d = cd_apply(
            &mut dice,
            "create",
            &json!({"sides": 20, "start": "low"}),
            &ctx(),
        )
        .unwrap();
        assert_eq!(d["value"], json!(1));
        // valeur explicite bornée
        let d = cd_apply(
            &mut dice,
            "create",
            &json!({"sides": 4, "value": 9}),
            &ctx(),
        )
        .unwrap();
        assert_eq!(d["value"], json!(4));
        assert!(cd_apply(&mut dice, "create", &json!({"sides": 1}), &ctx()).is_err());
        assert!(cd_apply(&mut dice, "create", &json!({"sides": 101}), &ctx()).is_err());
        assert!(
            cd_apply(
                &mut dice,
                "create",
                &json!({"sides": 6, "color": "red"}),
                &ctx()
            )
            .is_err()
        );
        assert!(
            cd_apply(
                &mut dice,
                "create",
                &json!({"sides": 6, "auto_trigger": "tick"}),
                &ctx()
            )
            .is_err()
        );
        assert!(cd_apply(&mut dice, "create", &json!({}), &ctx()).is_err());
    }

    #[test]
    fn cd_actions_sur_un_de() {
        let mut dice = vec![];
        cd_apply(
            &mut dice,
            "create",
            &json!({"sides": 6, "label": "Alarme"}),
            &ctx(),
        )
        .unwrap();
        // par libellé, insensible à la casse
        let d = cd_apply(
            &mut dice,
            "delta",
            &json!({"die": "alarme", "delta": -2}),
            &ctx(),
        )
        .unwrap();
        assert_eq!(d["value"], json!(4));
        assert_eq!(d["previousValue"], json!(6));
        let d = cd_apply(
            &mut dice,
            "delta",
            &json!({"die": "ID000000000000AA", "delta": -10}),
            &ctx(),
        )
        .unwrap();
        assert_eq!(d["value"], json!(1));
        assert!(
            cd_apply(
                &mut dice,
                "set_value",
                &json!({"die": "Alarme", "value": 7}),
                &ctx()
            )
            .is_err()
        );
        cd_apply(
            &mut dice,
            "set_value",
            &json!({"die": "Alarme", "value": 5}),
            &ctx(),
        )
        .unwrap();
        // réduire la taille borne la valeur
        let d = cd_apply(&mut dice, "configure", &json!({"die": "Alarme", "sides": 4, "auto_enabled": true, "auto_direction": "increase"}), &ctx()).unwrap();
        assert_eq!(
            (
                d["sides"].clone(),
                d["value"].clone(),
                d["autoDirection"].clone()
            ),
            (json!(4), json!(4), json!("increase"))
        );
        let d = cd_apply(&mut dice, "hide", &json!({"die": "Alarme"}), &ctx()).unwrap();
        assert_eq!(d["visible"], json!(false));
        let c = CdContext {
            active_scene: Some("SCENE".into()),
            ..ctx()
        };
        let d = cd_apply(&mut dice, "lock_scene", &json!({"die": "Alarme"}), &c).unwrap();
        assert_eq!(d["sceneId"], json!("SCENE"));
        let d = cd_apply(&mut dice, "unlock_scene", &json!({"die": "Alarme"}), &c).unwrap();
        assert!(d["sceneId"].is_null());
        assert!(
            cd_apply(
                &mut dice,
                "delta",
                &json!({"die": "inconnu", "delta": 1}),
                &ctx()
            )
            .is_err()
        );
        cd_apply(&mut dice, "remove", &json!({"die": "Alarme"}), &ctx()).unwrap();
        assert!(dice.is_empty());
    }

    #[test]
    fn cd_libelle_ambigu_et_clear() {
        let mut dice = vec![];
        let a = CdContext {
            new_id: "A".into(),
            ..Default::default()
        };
        let b = CdContext {
            new_id: "B".into(),
            ..Default::default()
        };
        cd_apply(&mut dice, "create", &json!({"sides": 6, "label": "X"}), &a).unwrap();
        cd_apply(&mut dice, "create", &json!({"sides": 6, "label": "x"}), &b).unwrap();
        assert!(cd_apply(&mut dice, "show", &json!({"die": "X"}), &a).is_err());
        assert!(cd_apply(&mut dice, "show", &json!({"die": "B"}), &a).is_ok());
        let r = cd_apply(&mut dice, "clear", &json!({}), &a).unwrap();
        assert_eq!(r["removed"], json!(2));
    }

    #[test]
    fn cd_normalisation_du_setting() {
        let s = json!({"dice": [
            {"id": "a", "sides": 6, "value": 99},
            {"id": "b", "sides": 1, "value": 1},
            {"id": "c", "sides": 8}
        ]});
        let d = cd_dice(Some(&s));
        assert_eq!(d.len(), 2);
        assert_eq!(d[0]["value"], json!(6));
        assert_eq!(d[1]["value"], json!(1));
        assert_eq!(d[1]["autoTrigger"], json!("turn"));
        assert!(cd_dice(None).is_empty());
    }

    #[test]
    fn ls_selection_et_tags() {
        let l1 = json!({"_id": "L1", "hidden": false, "flags": {"tagger": {"tags": ["Taverne", "feu"]}}});
        let l2 =
            json!({"_id": "L2", "hidden": true, "flags": {"tagger": {"tags": "rue, lampadaire"}}});
        let l3 = json!({"_id": "L3", "hidden": false, "flags": {}});
        assert_eq!(tagger_tags(&l2), vec!["rue", "lampadaire"]);
        let s = LightSelector::from_args(&json!({"tags": ["taverne"]})).unwrap();
        assert!(s.matches(&l1) && !s.matches(&l2) && !s.matches(&l3));
        let s = LightSelector::from_args(&json!({"tag_contains": "LAMP"})).unwrap();
        assert!(!s.matches(&l1) && s.matches(&l2));
        let s = LightSelector::from_args(&json!({"ids": ["L3"]})).unwrap();
        assert!(s.matches(&l3) && !s.matches(&l1));
        assert!(
            LightSelector::from_args(&json!({"all": true}))
                .unwrap()
                .matches(&l3)
        );
        assert!(LightSelector::from_args(&json!({})).is_err());
        assert_eq!(ls_summary(&l2)["state"], json!("off"));
    }

    #[test]
    fn ls_flags_de_decor() {
        let f = ls_graphic_flags(Some(
            &json!({"preset": "candle", "scale": 2, "speed": 0.75}),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            f["lightGraphicPath"],
            json!("modules/wgtngm-light-switch/media/candle.webm")
        );
        assert_eq!(f["lightGraphicEnabled"], json!(true));
        assert_eq!(f["lightGraphicSpeed"], json!(0.75));
        let f = ls_graphic_flags(Some(&json!("none"))).unwrap().unwrap();
        assert_eq!(f["lightGraphicEnabled"], json!(false));
        assert!(ls_graphic_flags(Some(&json!({"preset": "Inconnu"}))).is_err());
        assert!(ls_graphic_flags(Some(&json!({"path": "x.webm", "scale": 50}))).is_err());
        assert!(ls_graphic_flags(Some(&json!({"path": "x.webm", "speed": 3}))).is_err());
        assert!(ls_graphic_flags(None).unwrap().is_none());
        // une chaîne non « none » = nom de preset
        assert!(
            ls_graphic_flags(Some(&json!("Large Fire")))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn mp_flags_de_scene() {
        let (u, removed) = mp_scene_update(&json!({"tags": ["Combat", " forêt ", "combat"], "preset": "Taverne", "stop_all_sounds_on_start": true})).unwrap();
        assert_eq!(
            u["flags.wgtgm-mini-player.scenePlayTags"],
            json!("combat, forêt")
        );
        assert_eq!(
            u["flags.wgtgm-mini-player.scenePlayPreset"],
            json!("Taverne")
        );
        assert_eq!(
            u["flags.wgtgm-mini-player.sceneStopAllSoundsOnStart"],
            json!(true)
        );
        assert!(removed.is_empty());
        let (u, removed) = mp_scene_update(&json!({"clear": true, "tags": "a"})).unwrap();
        assert_eq!(removed.len(), 5);
        assert!(u.contains_key("flags.wgtgm-mini-player.-=scenePlayPreset"));
        assert!(!u.contains_key("flags.wgtgm-mini-player.-=scenePlayTags"));
        let (u, _) = mp_scene_update(&json!({"clear": ["preset"]})).unwrap();
        assert_eq!(u.len(), 1);
        assert!(mp_scene_update(&json!({"clear": ["nope"]})).is_err());
        assert!(mp_scene_update(&json!({"stop_all_sounds_on_start": "yes"})).is_err());
    }

    #[test]
    fn mp_presets_tries() {
        let s = json!({"k2": {"name": "Zeta", "tracks": [{"path": "a.ogg"}]}, "k1": {"tracks": []}, "k3": {"name": " "}});
        let p = mp_presets(Some(&s));
        assert_eq!(p.len(), 2);
        assert_eq!(p[0]["name"], json!("Zeta"));
        assert_eq!(p[1]["name"], json!("k1"));
        assert_eq!(p[0]["trackCount"], json!(1));
    }

    #[test]
    fn mrt_quantites() {
        let one = || 0.0; // chaque dé fait 1
        assert_eq!(mrt_roll_quantity("3", one).0, 3);
        assert_eq!(mrt_roll_quantity("1d4+1", one).0, 2);
        assert_eq!(mrt_roll_quantity("2d6 - 1", one).0, 1);
        assert_eq!(mrt_roll_quantity("d6", || 0.999).0, 6);
        assert_eq!(mrt_roll_quantity("1d1-5", one).0, 0); // plancher à 0
        let (q, w) = mrt_roll_quantity("500", one);
        assert_eq!(q, MRT_MAX_QUANTITY);
        assert!(w.is_none());
        let (q, w) = mrt_roll_quantity("1d4*2", one);
        assert_eq!(q, 1);
        assert!(w.is_some());
        let (q, w) = mrt_roll_quantity("1000d6", one);
        assert_eq!(q, 1);
        assert!(w.is_some());
        assert_eq!(
            mrt_quantity_formula(&json!({"flags": {"mini-rolltable": {"quantity": "1d4"}}})),
            Some("1d4".into())
        );
        assert_eq!(
            mrt_quantity_formula(&json!({"flags": {"mini-rolltable": {"quantity": "1"}}})),
            None
        );
        assert_eq!(
            mrt_quantity_formula(&json!({"flags": {"mini-rolltable": {"quantity": 2}}})),
            Some("2".into())
        );
        assert_eq!(mrt_quantity_formula(&json!({})), None);
    }

    #[test]
    fn mrt_tables_liees() {
        let r = json!({"type": "document", "documentUuid": "RollTable.abc"});
        assert_eq!(
            mrt_linked_table(&r),
            Some(("RollTable.abc".into(), Some("abc".into())))
        );
        let r = json!({"type": "document", "documentUuid": "Compendium.x.y.RollTable.abc"});
        assert_eq!(mrt_linked_table(&r).unwrap().1, None);
        assert!(
            mrt_linked_table(&json!({"type": "document", "documentUuid": "Item.abc"})).is_none()
        );
        assert!(
            mrt_linked_table(&json!({"type": "text", "documentUuid": "RollTable.abc"})).is_none()
        );
        let legacy = json!({"type": 1, "documentCollection": "RollTable", "documentId": "zz"});
        assert_eq!(mrt_linked_table(&legacy).unwrap().1, Some("zz".into()));
    }

    #[test]
    fn mrt_rendu_imbrique() {
        let draws = vec![
            json!({"roll": 2, "text": "Coffre", "quantity": 2, "quantityFormula": "2",
            "subDraws": [{"roll": 1, "text": "Or"}]}),
        ];
        let h = mrt_render_draws(&draws, 1, true);
        assert!(
            h.contains("×2")
                && h.contains("<ul><li><strong>1</strong> → Or</li></ul>")
                && h.contains("(dont +1)")
        );
    }
}
