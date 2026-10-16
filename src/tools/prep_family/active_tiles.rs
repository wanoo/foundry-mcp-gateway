//! Monk's Active Tiles — création et édition de tuiles-actions (mat_*).
//!
//! Une tuile active = une Tile ordinaire + `flags["monks-active-tiles"]`
//! ({active, trigger[], restriction, controlled, chance, pertoken, record,
//! minrequired, cooldown, actions:[{id, action, data}], history}). MAT ne
//! normalise rien à la création : une écriture serveur suffit, aucun
//! compagnon n'est nécessaire (le déclenchement manuel reste
//! `client_mat_trigger`, dans addons.rs).
//!
//! Garde-fou : toute liste d'actions passe par `script_guard::check_mat_actions`
//! AVANT l'écriture, pour renvoyer une erreur qui dit quoi corriger (le refus
//! de `modify_document` reste le filet de sécurité). La liste est toujours
//! écrite ENTIÈRE : une clé aplatie partielle serait refusée.

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::foundry::script_guard::{COMPANION, Need, check_mat_actions};
use crate::mcp::McpState;
use crate::tools::ToolDef;
use crate::tools::cc_family::mini_calendar::{ensure_module_active, read_setting};
use crate::tools::companion::Delivery;
use crate::tools::{str_arg, text_response};

const MODULE: &str = "monks-active-tiles";
const LABEL: &str = "Monk's Active Tiles";
/// Icône neutre du cœur (la tuile est cachée par défaut, elle ne se voit pas).
const DEFAULT_IMG: &str = "icons/svg/cube.svg";

/// Déclencheurs de MAT 14 (triggerModes) et ce qu'ils veulent dire.
#[rustfmt::skip]
const TRIGGERS: &[(&str, &str)] = &[
    ("enter", "a token moves INTO the tile (movement triggers need at least one action)"),
    ("exit", "a token moves OUT of the tile"),
    ("movement", "a token moves while inside the tile"),
    ("stop", "a token ends its movement inside the tile"),
    ("elevation", "a token inside the tile changes elevation"),
    ("rotation", "a token inside the tile rotates"),
    ("click", "a user left-clicks the tile (clickable even when hidden for the GM)"),
    ("rightclick", "a user right-clicks the tile"),
    ("dblclick", "a user double-clicks the tile"),
    ("dblrightclick", "a user double-right-clicks the tile"),
    ("create", "a token is created (dropped) on the tile"),
    ("hoverin", "the mouse pointer enters the tile"),
    ("hoverout", "the mouse pointer leaves the tile"),
    ("combatstart", "a combat starts on the scene"),
    ("round", "a new combat round starts"),
    ("turn", "a combatant's turn starts"),
    ("turnend", "a combatant's turn ends"),
    ("combatend", "the combat ends"),
    ("ready", "the scene's canvas finishes loading"),
    ("manual", "only fired by hand: the tile HUD button, another tile's `trigger` action, or client_mat_trigger"),
    ("door", "a door wall linked to this tile is opened/closed"),
    ("darkness", "the scene darkness reaches a value"),
    ("lighting", "the scene darkness changes"),
    ("time", "the world time reaches a value"),
];

/// Anciens déclencheurs composites, développés comme le fait MAT (getTrigger).
const LEGACY_TRIGGERS: &[(&str, &[&str])] = &[
    ("both", &["enter", "exit"]),
    ("hover", &["hoverin", "hoverout"]),
];

const WHO: &[&str] = &["all", "player", "gm"];

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

pub const COMMANDS: &[(&str, &str, Delivery)] = &[];

pub const READ_ONLY: &[&str] = &["mat_get_tile", "mat_action_types"];

/// Propriétés communes à la création et à la mise à jour.
fn tile_properties(create: bool) -> Value {
    let mut p = json!({
        "scene":{"type":"string","description":"Scene _id or name (default: the active scene)."},
        "name":{"type":"string","description":"Tile name (shown by MAT and mat_list; used to find the tile again)."},
        "x":{"type":"number","description":"Left edge in pixels."},
        "y":{"type":"number","description":"Top edge in pixels."},
        "width":{"type":"number","description":"Width in pixels."},
        "height":{"type":"number","description":"Height in pixels."},
        "grid":{"type":"object","description":"Position in grid squares instead of pixels (converted with the scene's grid size): col,row = top-left square (0-based), w,h = size in squares (default 1).",
            "properties":{"col":{"type":"number"},"row":{"type":"number"},"w":{"type":"number"},"h":{"type":"number"}}},
        "hidden":{"type":"boolean","description":"Hidden from players (an invisible trap still fires). Default true on create."},
        "img":{"type":"string","description":"Tile image path (default a neutral core icon)."},
        "trigger":{"description":"Trigger(s): one name or a list — see mat_action_types (enter, exit, click, manual, turn…). Unknown names are refused.",
            "anyOf":[{"type":"string"},{"type":"array","items":{"type":"string"}}]},
        "restriction":{"type":"string","enum":WHO,"description":"Whose TOKENS can fire it: all, player (player-owned tokens), gm (GM-owned)."},
        "controlled":{"type":"string","enum":WHO,"description":"Which USERS can fire it: all, player, gm."},
        "chance":{"type":"number","description":"Percent chance (0-100) to fire — only applies to movement triggers."},
        "once_per_token":{"type":"boolean","description":"Fire once per token (sets pertoken + record: the trigger history is kept)."},
        "record":{"type":"boolean","description":"Keep a trigger history (needed for once_per_token / minrequired / triggercount)."},
        "cooldown":{"type":"number","description":"Seconds before it can fire again."},
        "minrequired":{"type":"integer","description":"Fire only after this many recorded triggers."},
        "allowpaused":{"type":"boolean","description":"Players can fire it while the game is paused."},
        "active":{"type":"boolean","description":"Enabled (default true on create)."},
    });
    if create {
        p["actions"] = json!({"type":"array","description":"Ordered action list: [{action, data, id?}] — ids (16 alphanumerics) are generated when missing. See mat_action_types.",
            "items":{"type":"object","properties":{"action":{"type":"string"},"data":{"type":"object"},"id":{"type":"string"}},"required":["action"]}});
        p["region"] = json!({"type":"boolean","description":"Foundry v14: also create a Region covering the tile with a « monks-active-tiles.triggerTile » behavior (tokenEnter/tokenExit relayed as enter/exit; MAT then acts on the tokens the user CONTROLS, and ignores the tile's trigger list/restriction/chance). Deleting the tile does not delete the region. Default false."});
    } else {
        p["tile"] = json!({"type":"string","description":"Tile _id or name."});
        p["actions"] = json!({"type":"array","description":"REPLACES the whole action list.","items":{"type":"object"}});
        p["append_actions"] = json!({"type":"array","description":"Actions added at the end of the list.","items":{"type":"object"}});
        p["remove_action_ids"] = json!({"type":"array","items":{"type":"string"},"description":"Action ids to remove (see mat_get_tile)."});
        p["reset_history"] = json!({"type":"boolean","description":"Clear the trigger history (once-per-token tiles fire again)."});
    }
    p
}

const GUARD_NOTE: &str = "The gateway refuses actions that run code: runcode, runbatch, url, dialog and unknown/third-party actions; runmacro needs the GM's « Remote script-macro execution » setting on auto; values that MAT evaluates as JavaScript must be a number (« 5 »), operator+number (« + 3 », « = 0 », « > 2 »), a die roll (« - [[1d6]] »), true/false or an identifier/UUID; texts must not contain « {{ » (Handlebars), active HTML, or start with « = », « + », « - ». Only the GM can lift this, in Foundry (companion setting « Remote script execution » = auto).";

#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    let create_desc: &'static str = Box::leak(format!(
        "Monk's Active Tiles: create an ACTIVE tile (a trap, pressure plate, button…) on a scene (default: active scene). \
         Position in pixels (x, y, width, height) or in grid squares (grid {{col,row,w,h}}). Hidden by default (invisible trap — it still fires). Movement triggers (enter/exit/stop…, and the optional v14 `region`) fire when a token is moved IN A CLIENT (a player or the GM dragging it) — NOT when the gateway moves it with move_token/update_token; use client_mat_trigger to fire a tile from the gateway. \
         `actions` = ordered [{{action, data}}] — call mat_action_types first to get valid action names and data fields. \
         Triggers are validated (unknown ones refused). Returns the tile _id/uuid and the action ids. \
         Note: `hurtheal` damages the system's primary token attribute (attributes.hp) — WRONG for starwarsffg (wounds count up): use `alter` on system.stats.wounds.value instead. \
         {GUARD_NOTE}"
    ).into_boxed_str());
    let update_desc: &'static str = Box::leak(format!(
        "Monk's Active Tiles: modify an active tile by _id or name (scene: id/name, default active scene). \
         Any of the create fields; actions: `actions` replaces the list, `append_actions` adds at the end, `remove_action_ids` removes by id \
         (the whole list is re-validated and re-written, existing actions included); `reset_history` clears the trigger history; `active:false` disables it. \
         {GUARD_NOTE}"
    ).into_boxed_str());
    vec![
        ("mat_create_tile", create_desc,
         json!({"type":"object","properties":tile_properties(true),"required":["name"]})),
        ("mat_update_tile", update_desc,
         json!({"type":"object","properties":tile_properties(false),"required":["tile"]})),
        ("mat_get_tile",
         "Monk's Active Tiles: full readable configuration of one active tile (by _id or name; scene id/name, default active scene) — geometry, triggers, conditions (restriction, controlled, chance, once-per-token, cooldown, minrequired), actions with id/action/data, trigger history per token, and whether the companion would refuse to run it (code-running actions). Read-only.",
         json!({"type":"object","properties":{
            "scene":{"type":"string","description":"Scene _id or name (default: the active scene)."},
            "tile":{"type":"string","description":"Tile _id or name."}},"required":["tile"]})),
        ("mat_action_types",
         "Monk's Active Tiles: static catalogue for writing valid tiles — the triggers and their meaning, the SAFE actions with their data fields (types, allowed values, defaults, examples), the forms of entity/location/recipient references, and what the gateway's code guard refuses (runcode, runmacro, url, dialog, evaluated values, {{…}} templates…) and why. `action` returns a single action. Read-only, no Foundry call.",
         json!({"type":"object","properties":{
            "action":{"type":"string","description":"Only this action (e.g. « chatmessage »)."}}})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    match name {
        "mat_action_types" => action_types(args),
        "mat_create_tile" => create_tile(state, args).await,
        "mat_update_tile" => update_tile(state, args).await,
        "mat_get_tile" => get_tile(state, args).await,
        _ => bail!("Unknown tool: {name}"),
    }
}

// ------------------------------------------------------------ helpers purs

/// Identifiant façon foundry.utils.randomID (16 caractères alphanumériques).
fn random_id() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    (0..16)
        .map(|_| CHARS[rand::random_range(0..CHARS.len())] as char)
        .collect()
}

/// Déclencheur(s) : chaîne ou liste → liste validée (sans doublons).
fn parse_triggers(v: &Value) -> Result<Vec<String>> {
    let raw: Vec<&str> = match v {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .ok_or_else(|| anyhow!("'trigger' must be a string or a list of strings"))
            })
            .collect::<Result<_>>()?,
        _ => bail!("'trigger' must be a string or a list of strings"),
    };
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: &str| {
        if !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    };
    for t in raw {
        let t = t.trim();
        if let Some((_, parts)) = LEGACY_TRIGGERS.iter().find(|(k, _)| *k == t) {
            parts.iter().for_each(|p| push(p));
        } else if TRIGGERS.iter().any(|(k, _)| *k == t) {
            push(t);
        } else {
            let known: Vec<&str> = TRIGGERS.iter().map(|(k, _)| *k).collect();
            bail!(
                "unknown trigger « {t} ». Valid triggers: {} (see mat_action_types).",
                known.join(", ")
            );
        }
    }
    if out.is_empty() {
        bail!("'trigger' is empty — give at least one trigger (e.g. « enter » or « manual »)");
    }
    Ok(out)
}

/// Normalise une liste d'actions : objet {action, data, id} ; id généré s'il
/// manque ; ids uniques. `taken` = ids déjà présents (ajout en fin de liste).
fn normalize_actions(v: &Value, taken: &[String]) -> Result<Vec<Value>> {
    let list = v
        .as_array()
        .ok_or_else(|| anyhow!("actions must be a list of {{action, data}} objects"))?;
    let mut ids: Vec<String> = taken.to_vec();
    let mut out = Vec::with_capacity(list.len());
    for (i, a) in list.iter().enumerate() {
        let o = a
            .as_object()
            .ok_or_else(|| anyhow!("action #{i} must be an object {{action, data}}"))?;
        let kind = o
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow!("action #{i}: 'action' (the action type, e.g. « chatmessage ») is required")
            })?;
        let data = match o.get("data") {
            None | Some(Value::Null) => json!({}),
            Some(d @ Value::Object(_)) => d.clone(),
            Some(_) => bail!("action #{i} « {kind} »: 'data' must be an object"),
        };
        let id = match o.get("id").and_then(Value::as_str).map(str::trim) {
            Some(s) if !s.is_empty() => {
                if !s.chars().all(|c| c.is_ascii_alphanumeric()) {
                    bail!(
                        "action #{i} « {kind} »: id « {s} » must be alphanumeric (or omit it to get one)"
                    );
                }
                s.to_string()
            }
            _ => loop {
                let r = random_id();
                if !ids.contains(&r) {
                    break r;
                }
            },
        };
        if ids.contains(&id) {
            bail!("action #{i} « {kind} »: duplicate id « {id} »");
        }
        ids.push(id.clone());
        out.push(json!({"id": id, "action": kind, "data": data}));
    }
    Ok(out)
}

/// Conseils de réécriture selon ce que le garde-fou a relevé.
fn guard_hints(reason: &str) -> Vec<&'static str> {
    let mut h = Vec::new();
    if reason.contains("runs code or is not a known safe action") {
        h.push("use only the safe actions listed by mat_action_types — runcode, runbatch, url, dialog and third-party actions run JavaScript (to run a GM-approved macro, use client_run_macro instead)");
    }
    if reason.contains("Handlebars") {
        h.push("write plain text without « {{ … }} » (MAT compiles it with Handlebars, prototype access allowed — that is code)");
    }
    if reason.contains("starts with an operator") {
        h.push("don't start a text with « = », « + » or « - » (MAT then evaluates it as JavaScript) — rephrase or put a word first");
    }
    if reason.contains("active HTML") {
        h.push("remove <script>/<iframe>/<object>/<embed>, on…= handlers and javascript: links — plain text or simple formatting only");
    }
    if reason.contains("would evaluate it as JavaScript") {
        h.push("value fields take a number (« 5 »), an operator and a number (« + 3 », « = 0 », « > 2 »), a die roll (« - [[1d6]] »), true/false, or an identifier/UUID/path without spaces — no strings in quotes, no function calls, no expressions");
    }
    if reason.contains("runmacro") {
        h.push("« runmacro » needs the GM's « Remote script-macro execution » setting on auto; otherwise trigger the macro with client_run_macro (asks the GM)");
    }
    if reason.contains("unreadable") {
        h.push("actions must be a list of {action, data} objects");
    }
    h
}

/// Message d'erreur complet : ce qui est refusé, pourquoi, et comment réécrire.
fn guard_error(need: &Need, script_mode: &str, macro_mode: &str) -> String {
    let (setting, mode) = match need {
        Need::MacroScript(_) => ("Remote script-macro execution", macro_mode),
        _ => ("Remote script execution", script_mode),
    };
    let hints = guard_hints(need.reason());
    let mut msg = format!(
        "Refused before writing: {}. These actions would run code in the GM's browser when the tile fires; \
         the companion setting « {setting} » is « {mode} » and only « auto » (set by the GM in Foundry, never \
         through the gateway) allows it.",
        need.reason()
    );
    if !hints.is_empty() {
        msg.push_str(" How to write it instead: ");
        msg.push_str(&hints.join(" ; "));
        msg.push('.');
    }
    msg
}

fn bool_arg(args: &Value, k: &str) -> Result<Option<bool>> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => bail!("'{k}' must be true or false"),
    }
}

fn num_arg(args: &Value, k: &str) -> Result<Option<f64>> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .map(Some)
            .ok_or_else(|| anyhow!("'{k}' must be a number")),
    }
}

/// Réglages MAT (hors actions) demandés par l'appel, en clés relatives au flag.
fn option_flags(args: &Value) -> Result<Map<String, Value>> {
    let mut f = Map::new();
    if let Some(t) = args.get("trigger").filter(|v| !v.is_null()) {
        f.insert("trigger".into(), json!(parse_triggers(t)?));
    }
    for k in ["restriction", "controlled"] {
        if let Some(s) = str_arg(args, k) {
            if !WHO.contains(&s.as_str()) {
                bail!("'{k}' must be one of: all, player, gm (got « {s} »)");
            }
            f.insert(k.into(), json!(s));
        }
    }
    if let Some(c) = num_arg(args, "chance")? {
        if !(0.0..=100.0).contains(&c) {
            bail!("'chance' is a percentage between 0 and 100");
        }
        f.insert("chance".into(), json!(c));
    }
    if let Some(c) = num_arg(args, "cooldown")? {
        if c < 0.0 {
            bail!("'cooldown' is a number of seconds ≥ 0");
        }
        f.insert("cooldown".into(), json!(c));
    }
    if let Some(m) = num_arg(args, "minrequired")? {
        if m < 0.0 || m.fract() != 0.0 {
            bail!("'minrequired' is a whole number ≥ 0");
        }
        f.insert("minrequired".into(), json!(m as i64));
    }
    for k in ["active", "allowpaused", "record"] {
        if let Some(b) = bool_arg(args, k)? {
            f.insert(k.into(), json!(b));
        }
    }
    if let Some(b) = bool_arg(args, "once_per_token")? {
        // pertoken ne marche QUE si l'historique est tenu (record === true).
        f.insert("pertoken".into(), json!(b));
        if b {
            f.insert("record".into(), json!(true));
        }
    }
    Ok(f)
}

/// Taille d'une case : `grid.size` (v12+) ou ancien nombre, défaut 100.
fn grid_size(scene: &Value) -> f64 {
    scene
        .get("grid")
        .and_then(|g| g.get("size").and_then(Value::as_f64).or_else(|| g.as_f64()))
        .filter(|s| *s > 0.0)
        .unwrap_or(100.0)
}

/// Géométrie demandée (pixels ou cases) → champs x/y/width/height.
fn geometry(args: &Value, size: f64) -> Result<Map<String, Value>> {
    let mut g = Map::new();
    if let Some(grid) = args.get("grid").filter(|v| !v.is_null()) {
        if ["x", "y", "width", "height"]
            .iter()
            .any(|k| args.get(*k).is_some_and(|v| !v.is_null()))
        {
            bail!("give either pixels (x, y, width, height) or 'grid', not both");
        }
        let n = |k: &str, d: Option<f64>| -> Result<f64> {
            match grid.get(k) {
                None | Some(Value::Null) => {
                    d.ok_or_else(|| anyhow!("'grid.{k}' is required (grid {{col,row,w,h}})"))
                }
                Some(v) => v
                    .as_f64()
                    .ok_or_else(|| anyhow!("'grid.{k}' must be a number")),
            }
        };
        let (col, row) = (n("col", None)?, n("row", None)?);
        let (w, h) = (n("w", Some(1.0))?, n("h", Some(1.0))?);
        if w <= 0.0 || h <= 0.0 {
            bail!("'grid.w' and 'grid.h' must be > 0");
        }
        g.insert("x".into(), json!((col * size).round()));
        g.insert("y".into(), json!((row * size).round()));
        g.insert("width".into(), json!((w * size).round()));
        g.insert("height".into(), json!((h * size).round()));
        return Ok(g);
    }
    for k in ["x", "y", "width", "height"] {
        if let Some(v) = num_arg(args, k)? {
            if (k == "width" || k == "height") && v <= 0.0 {
                bail!("'{k}' must be > 0");
            }
            g.insert(k.into(), json!(v));
        }
    }
    Ok(g)
}

/// Tuile visée dans une scène : par _id, sinon par nom (champ ou flag MAT).
fn find_tile<'a>(scene: &'a Value, ident: &str) -> Result<&'a Value> {
    let tiles = scene
        .get("tiles")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if let Some(t) = tiles.iter().find(|t| t["_id"].as_str() == Some(ident)) {
        return Ok(t);
    }
    let by_name: Vec<&Value> = tiles
        .iter()
        .filter(|t| {
            t["name"].as_str() == Some(ident)
                || t.pointer("/flags/monks-active-tiles/name")
                    .and_then(Value::as_str)
                    == Some(ident)
        })
        .collect();
    match by_name.as_slice() {
        [one] => Ok(one),
        [] => bail!(
            "tile « {ident} » not found on scene « {} » (use mat_list for the active tiles' _id and names)",
            scene["name"].as_str().unwrap_or("?")
        ),
        many => bail!(
            "{} tiles are named « {ident} » on this scene — pass the _id: {}",
            many.len(),
            many.iter()
                .filter_map(|t| t["_id"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn tile_uuid(scene: &Value, tile: &Value) -> String {
    format!(
        "Scene.{}.Tile.{}",
        scene["_id"].as_str().unwrap_or(""),
        tile["_id"].as_str().unwrap_or("")
    )
}

/// Historique MAT → déclenchements par token (nom résolu sur la scène).
fn history_summary(mat: &Value, scene: &Value) -> Value {
    let Some(h) = mat.get("history").and_then(Value::as_object) else {
        return json!({"total": 0, "by_token": []});
    };
    let tokens = scene
        .get("tokens")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut total = 0;
    let by_token: Vec<Value> = h
        .iter()
        .map(|(k, v)| {
            let trig = v
                .get("triggered")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            total += trig.len();
            let name = if k == "_blank" {
                Some("(no token)".to_string())
            } else {
                tokens
                    .iter()
                    .find(|t| t["_id"].as_str() == Some(k))
                    .and_then(|t| t["name"].as_str().map(String::from))
            };
            json!({
                "token": k, "name": name, "count": trig.len(),
                "last": trig.last().map(|e| json!({"how": e.get("how"), "who": e.get("who"), "when": e.get("when")})),
            })
        })
        .collect();
    json!({"total": total, "by_token": by_token})
}

/// Vue lisible d'une tuile active.
fn tile_view(scene: &Value, tile: &Value) -> Value {
    let empty = json!({});
    let mat = tile.pointer("/flags/monks-active-tiles").unwrap_or(&empty);
    let actions = mat.get("actions").cloned().unwrap_or_else(|| json!([]));
    let guard = check_mat_actions(&actions).map(|n| match n {
        Need::MacroScript(r) => json!({"needs": "runMacroScriptMode=auto", "reason": r}),
        other => json!({"needs": "runScriptMode=auto", "reason": other.reason()}),
    });
    let trigger = match mat.get("trigger") {
        Some(Value::String(s)) => json!([s]),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    let pick = |k: &str| mat.get(k).cloned().unwrap_or(Value::Null);
    json!({
        "scene": {"_id": scene["_id"], "name": scene["name"]},
        "tile": {
            "_id": tile["_id"], "uuid": tile_uuid(scene, tile),
            "name": tile.get("name").filter(|v| !v.is_null()).or_else(|| mat.get("name")),
            "x": tile["x"], "y": tile["y"], "width": tile["width"], "height": tile["height"],
            "hidden": tile["hidden"], "img": tile.pointer("/texture/src"),
        },
        "is_active_tile": tile.pointer("/flags/monks-active-tiles").is_some(),
        "active": pick("active"),
        "trigger": trigger,
        "conditions": {
            "restriction": pick("restriction"), "controlled": pick("controlled"),
            "chance": pick("chance"), "once_per_token": mat.get("pertoken").and_then(Value::as_bool).unwrap_or(false),
            "record": pick("record"), "cooldown": pick("cooldown"), "minrequired": pick("minrequired"),
            "allowpaused": pick("allowpaused"),
        },
        "actions": actions,
        "history": history_summary(mat, scene),
        "companion_would_refuse": guard,
    })
}

// ------------------------------------------------------------ Foundry

/// Scène visée : `scene` (id ou nom) / scene_id / scene_name, sinon l'active.
async fn resolve_scene(state: &McpState, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    let wanted = str_arg(args, "scene")
        .or_else(|| str_arg(args, "scene_id"))
        .or_else(|| str_arg(args, "scene_name"));
    let scene = match &wanted {
        Some(s) => foundry.find_document("scenes", s, None).await?,
        None => {
            let w = json!({"active": true})
                .as_object()
                .cloned()
                .unwrap_or_default();
            foundry
                .get_documents("scenes", Some(&w), None, 0, Some(1))
                .await?
                .into_iter()
                .next()
        }
    };
    scene.ok_or_else(|| match wanted {
        Some(s) => anyhow!("scene « {s} » not found"),
        None => anyhow!("no active scene — pass 'scene' (id or name)"),
    })
}

/// Pré-validation des actions par le garde-fou, avec message lisible. Laisse
/// passer si le MJ a mis le réglage correspondant sur « auto » (le garde-fou
/// de modify_document l'accepterait aussi).
async fn guard_actions(state: &McpState, actions: &Value) -> Result<()> {
    let Some(need) = check_mat_actions(actions) else {
        return Ok(());
    };
    let mode = |v: Option<Value>, d: &str| {
        v.and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| d.to_string())
    };
    let script = mode(
        read_setting(&state.foundry, &format!("{COMPANION}.runScriptMode")).await?,
        "off",
    );
    let macro_mode = mode(
        read_setting(&state.foundry, &format!("{COMPANION}.runMacroScriptMode")).await?,
        "confirm",
    );
    let allowed = match &need {
        Need::Script(_) => script == "auto",
        Need::MacroScript(_) => macro_mode == "auto",
        Need::Forbidden(_) => false,
    };
    if allowed {
        Ok(())
    } else {
        bail!("{}", guard_error(&need, &script, &macro_mode))
    }
}

async fn create_tile(state: &McpState, args: &Value) -> Result<Value> {
    let name = str_arg(args, "name")
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow!("'name' is required"))?;
    ensure_module_active(&state.foundry, MODULE, LABEL).await?;
    let scene = resolve_scene(state, args).await?;
    let scene_id = scene["_id"].as_str().unwrap_or("").to_string();
    let size = grid_size(&scene);

    let geo = geometry(args, size)?;
    if !args.get("grid").is_some_and(|v| !v.is_null())
        && ["x", "y"].iter().any(|k| !geo.contains_key(*k))
    {
        bail!("position required: x and y in pixels, or grid {{col,row,w,h}} in squares");
    }
    let mut tile = json!({
        "name": name,
        "width": size, "height": size,
        "hidden": bool_arg(args, "hidden")?.unwrap_or(true),
        "texture": {"src": str_arg(args, "img").unwrap_or_else(|| DEFAULT_IMG.into())},
    });
    for (k, v) in geo {
        tile[k] = v;
    }

    let actions = normalize_actions(args.get("actions").unwrap_or(&json!([])), &[])?;
    let actions = Value::Array(actions);
    guard_actions(state, &actions).await?;

    // Défauts de l'interface MAT, puis ce que l'appel demande.
    let mut flags = json!({
        "name": name, "active": true, "trigger": ["enter"], "vision": true,
        "chance": 100, "restriction": "all", "controlled": "all",
        "pertoken": false, "minrequired": 0, "actions": actions,
    });
    for (k, v) in option_flags(args)? {
        flags[k] = v;
    }
    let movement = [
        "enter",
        "exit",
        "movement",
        "stop",
        "elevation",
        "rotation",
        "create",
    ];
    let empty_moving = flags["actions"].as_array().is_some_and(Vec::is_empty)
        && flags["trigger"].as_array().is_some_and(|t| {
            t.iter()
                .any(|x| movement.contains(&x.as_str().unwrap_or("")))
        });
    tile["flags"] = json!({ MODULE: flags });

    let res = state
        .foundry
        .modify_document(
            "Tile",
            "create",
            json!({"action": "create", "parentUuid": format!("Scene.{scene_id}"),
                   "renderSheet": false, "data": [tile]}),
        )
        .await?;
    let created = res
        .pointer("/result/0")
        .cloned()
        .ok_or_else(|| anyhow!("Foundry did not return the created tile: {res}"))?;
    let tile_id = created["_id"].as_str().unwrap_or("").to_string();
    let uuid = format!("Scene.{scene_id}.Tile.{tile_id}");

    let region = if bool_arg(args, "region")?.unwrap_or(false) {
        Some(create_region(state, &scene_id, &created, &uuid, &name).await?)
    } else {
        None
    };

    let mut out = json!({
        "created": {"_id": tile_id, "uuid": uuid, "name": name},
        "scene": {"_id": scene["_id"], "name": scene["name"]},
        "geometry": {"x": created["x"], "y": created["y"], "width": created["width"],
                     "height": created["height"], "grid_size": size},
        "hidden": created["hidden"],
        "trigger": created.pointer("/flags/monks-active-tiles/trigger"),
        "actions": created.pointer("/flags/monks-active-tiles/actions")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|x| json!({"id": x["id"], "action": x["action"]})).collect::<Vec<_>>()),
        "hint": "Fire it by hand with client_mat_trigger (tile must be on the GM's viewed scene); inspect with mat_get_tile.",
    });
    if empty_moving {
        out["warning"] = json!("movement triggers do nothing while the action list is empty");
    }
    if let Some(r) = region {
        out["region"] = r;
    }
    Ok(text_response(&out))
}

/// Region v14 couvrant la tuile, qui relaie entrée/sortie à la tuile.
async fn create_region(
    state: &McpState,
    scene_id: &str,
    tile: &Value,
    uuid: &str,
    name: &str,
) -> Result<Value> {
    let shape = json!({
        "type": "rectangle", "x": tile["x"], "y": tile["y"],
        "width": tile["width"], "height": tile["height"], "rotation": 0, "hole": false,
    });
    let region = json!({
        "name": name, "color": "#ff8800", "shapes": [shape],
        "behaviors": [{
            "name": format!("MAT: {name}"), "type": "monks-active-tiles.triggerTile",
            "system": {"events": ["tokenEnter", "tokenExit"], "uuid": uuid, "usetiletrigger": true},
        }],
    });
    let res = state
        .foundry
        .modify_document(
            "Region",
            "create",
            json!({"action": "create", "parentUuid": format!("Scene.{scene_id}"),
                   "renderSheet": false, "data": [region]}),
        )
        .await?;
    let r = res
        .pointer("/result/0")
        .ok_or_else(|| anyhow!("Foundry did not return the created region: {res}"))?;
    Ok(json!({"_id": r["_id"], "behaviors": r["behaviors"].as_array().map(Vec::len)}))
}

async fn update_tile(state: &McpState, args: &Value) -> Result<Value> {
    let ident = str_arg(args, "tile").ok_or_else(|| anyhow!("'tile' (_id or name) is required"))?;
    ensure_module_active(&state.foundry, MODULE, LABEL).await?;
    let scene = resolve_scene(state, args).await?;
    let tile = find_tile(&scene, &ident)?;
    let tile_id = tile["_id"].as_str().unwrap_or("").to_string();
    let empty = json!({});
    let mat = tile.pointer("/flags/monks-active-tiles").unwrap_or(&empty);

    let mut u = Map::new();
    u.insert("_id".into(), json!(tile_id));
    let mut m = Map::new();
    let mut flag_keys: Vec<String> = Vec::new();
    for (k, v) in geometry(args, grid_size(&scene))? {
        u.insert(k, v);
    }
    if let Some(n) = str_arg(args, "name") {
        u.insert("name".into(), json!(n));
        m.insert("name".into(), json!(n));
    }
    if let Some(h) = bool_arg(args, "hidden")? {
        u.insert("hidden".into(), json!(h));
    }
    if let Some(i) = str_arg(args, "img") {
        u.insert("texture.src".into(), json!(i));
    }
    for (k, v) in option_flags(args)? {
        m.insert(k, v);
    }

    // Actions : on recalcule la liste ENTIÈRE et on la réécrit.
    let replace = args.get("actions").filter(|v| !v.is_null());
    let append = args.get("append_actions").filter(|v| !v.is_null());
    let remove: Vec<String> = match args.get("remove_action_ids") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        Some(_) => bail!("'remove_action_ids' must be a list of action ids"),
    };
    let mut new_actions = None;
    if replace.is_some() || append.is_some() || !remove.is_empty() {
        let mut list: Vec<Value> = match replace {
            Some(r) => normalize_actions(r, &[])?,
            None => mat
                .get("actions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        };
        if !remove.is_empty() {
            let known: Vec<String> = list
                .iter()
                .filter_map(|a| a["id"].as_str().map(String::from))
                .collect();
            let missing: Vec<&String> = remove.iter().filter(|r| !known.contains(r)).collect();
            if !missing.is_empty() {
                bail!(
                    "unknown action id(s): {} — this tile's actions: {}",
                    missing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    known.join(", ")
                );
            }
            list.retain(|a| {
                !a["id"]
                    .as_str()
                    .is_some_and(|i| remove.iter().any(|r| r == i))
            });
        }
        if let Some(a) = append {
            let taken: Vec<String> = list
                .iter()
                .filter_map(|a| a["id"].as_str().map(String::from))
                .collect();
            list.extend(normalize_actions(a, &taken)?);
        }
        let list = Value::Array(list);
        guard_actions(state, &list).await?;
        m.insert("actions".into(), list.clone());
        new_actions = Some(list);
    }
    if bool_arg(args, "reset_history")?.unwrap_or(false) {
        m.insert("-=history".into(), Value::Null);
    }
    // Flags MAT en objet imbriqué (fusion récursive ; un tableau y est
    // remplacé en entier) : la clé aplatie « flags.monks-active-tiles.actions »
    // est mal lue par le garde-fou, qui la reparcourt élément par élément.
    if !m.is_empty() {
        let mut changed_flags: Vec<String> =
            m.keys().map(|k| format!("flags.{MODULE}.{k}")).collect();
        flag_keys.append(&mut changed_flags);
        u.insert("flags".into(), json!({ MODULE: Value::Object(m) }));
    }
    if u.len() == 1 {
        bail!(
            "nothing to change — pass at least one field (name, x/y/width/height/grid, hidden, img, trigger, restriction, controlled, chance, once_per_token, record, cooldown, minrequired, allowpaused, active, actions, append_actions, remove_action_ids, reset_history)"
        );
    }
    let changed: Vec<String> = u
        .keys()
        .filter(|k| *k != "_id" && *k != "flags")
        .cloned()
        .chain(flag_keys)
        .collect();
    state
        .foundry
        .modify_document(
            "Tile",
            "update",
            json!({"action": "update", "diff": false, "recursive": true, "render": true,
                   "parentUuid": format!("Scene.{}", scene["_id"].as_str().unwrap_or("")),
                   "updates": [Value::Object(u)]}),
        )
        .await?;
    Ok(text_response(&json!({
        "updated": {"_id": tile_id, "uuid": tile_uuid(&scene, tile)},
        "scene": {"_id": scene["_id"], "name": scene["name"]},
        "changed": changed,
        "actions": new_actions.as_ref().and_then(Value::as_array)
            .map(|a| a.iter().map(|x| json!({"id": x["id"], "action": x["action"]})).collect::<Vec<_>>()),
    })))
}

async fn get_tile(state: &McpState, args: &Value) -> Result<Value> {
    let ident = str_arg(args, "tile").ok_or_else(|| anyhow!("'tile' (_id or name) is required"))?;
    let scene = resolve_scene(state, args).await?;
    let tile = find_tile(&scene, &ident)?;
    Ok(text_response(&tile_view(&scene, tile)))
}

// ------------------------------------------------------------ catalogue

/// Formes de références communes (getEntities / getLocation / getForPlayers).
fn references() -> Value {
    json!({
        "entity": {
            "form": "{\"id\": \"<ref>\", \"name\": \"<label, optional>\"}",
            "refs": {
                "tile": "this tile", "token": "the token(s) that fired the tile",
                "within": "everything inside the tile's area (pick the collection)",
                "players": "the players' tokens", "controlled": "the tokens the user controls",
                "previous": "the result of the previous action (e.g. after exists/first)",
                "scene": "the active scene", "tagger:<tag>": "documents tagged with Tagger",
                "<UUID>": "a document UUID, e.g. Scene.<sid>.Token.<tid>, Scene.<sid>.AmbientLight.<lid>, Scene.<sid>.Tile.<tid>, Actor.<id>",
                "<UUID>,<UUID>": "several UUIDs separated by commas",
            },
        },
        "location": {
            "form": "{\"id\": \"<ref>\"} or {\"x\": 1200, \"y\": 800, \"sceneId\": \"<scene _id>\"}",
            "refs": {"tile": "this tile", "token": "the triggering token", "origin": "where the token started its move",
                     "previous": "previous action's location", "players": "the players' tokens",
                     "tagger:<tag>": "tagged placeable", "Scene.<sid>.Tile.<tid>": "another tile (teleport targets)"},
        },
        "recipients": {
            "fields": "showto / for / audiofor / panfor / pingfrom",
            "values": ["everyone", "players", "gm", "trigger (the user who fired it)", "token (owners of the token)", "owner", "previous", "<user _id>"],
        },
        "values": "Numbers/values: « 5 » sets, « + 3 » / « - 2 » (WITH a space) adjusts, « = 5 » sets explicitly, « > 0 » compares, « [[1d6]] » rolls, « - [[1d4]] » rolls and subtracts.",
    })
}

/// Actions sûres documentées : (clé, description, champs, exemple).
#[rustfmt::skip]
fn action_catalog() -> Vec<(&'static str, &'static str, Value, Value)> {
    vec![
        ("pause", "Pause / unpause the game.",
         json!({"pause": {"type":"string","values":["pause","unpause","toggle"],"default":"pause"}}),
         json!({"pause":"pause"})),
        ("delay", "Wait before the next action.",
         json!({"delay": {"type":"string","note":"seconds: « 5 », a range « 5-15 », or a list to pick from « 2,4,6 »","required":true}}),
         json!({"delay":"3"})),
        ("movement", "Stop the triggering token's movement.",
         json!({"snap": {"type":"boolean","note":"snap the token to the grid"}}),
         json!({})),
        ("chatmessage", "Post a chat message.",
         json!({"text": {"type":"string","required":true,"note":"plain text / simple HTML; no {{…}}, no leading = + -"},
                "flavor": {"type":"string"}, "entity": {"type":"entity","note":"speaker (default: none)"},
                "incharacter": {"type":"boolean"}, "chatbubble": {"type":"string","values":["true","false","bubble"],"default":"false"},
                "showto": {"type":"recipients","default":"everyone"}, "language": {"type":"string"}}),
         json!({"text":"A pressure plate clicks under your feet…","flavor":"Trap","showto":"everyone","chatbubble":"false"})),
        ("notification", "Show a UI notification.",
         json!({"text": {"type":"string","required":true}, "type": {"type":"string","values":["info","warning","error"],"default":"info"},
                "showto": {"type":"recipients","default":"trigger"}}),
         json!({"text":"You hear a distant rumble.","type":"info","showto":"everyone"})),
        ("hurtheal", "Damage (negative) or heal (positive) tokens. WARNING: changes game.system.primaryTokenAttribute (attributes.hp) — wrong for starwarsffg (wounds count UP): use `alter` on system.stats.wounds.value there.",
         json!({"entity": {"type":"entity","default":"token"}, "value": {"type":"value","required":true,"note":"« - [[1d4]] » damage, « + 5 » heal"},
                "chatMessage": {"type":"boolean","note":"post the roll"}, "rollmode": {"type":"string","values":["roll","gmroll","blindroll","selfroll"]},
                "showdice": {"type":"boolean","default":true}}),
         json!({"entity":{"id":"token"},"value":"- [[1d4]]","chatMessage":true,"rollmode":"roll"})),
        ("alter", "Change an attribute of documents (token/actor/tile/wall…).",
         json!({"entity": {"type":"entity","default":"token"},
                "collection": {"type":"string","values":["actors","drawings","items","journal","macros","playlists","scene","tiles","tokens","walls"],"default":"tokens","note":"needed with previous/within"},
                "attribute": {"type":"string","required":true,"note":"dotted path, e.g. system.stats.wounds.value, hidden, elevation"},
                "value": {"type":"value","required":true,"note":"« = 5 », « + 3 », « - [[1d4]] », « = true ». Strings in quotes are refused (they are evaluated)."},
                "chatMessage": {"type":"boolean"}, "rollmode": {"type":"string","values":["roll","gmroll","blindroll","selfroll"]}}),
         json!({"entity":{"id":"token"},"attribute":"system.stats.wounds.value","value":"+ [[1d4]]"})),
        ("showhide", "Show or hide tokens, tiles or drawings.",
         json!({"entity": {"type":"entity","note":"e.g. a UUID Scene.<sid>.Token.<tid>"}, "collection": {"type":"string","values":["tokens","tiles","drawings"],"default":"tokens"},
                "hidden": {"type":"string","values":["show","hide","toggle"],"default":"hide"}, "fade": {"type":"number","default":0,"note":"seconds"}}),
         json!({"entity":{"id":"Scene.SCENEID.Token.TOKENID"},"collection":"tokens","hidden":"show","fade":0.5})),
        ("activate", "Turn lights/sounds on or off (they become hidden/visible), or enable/disable active tiles.",
         json!({"entity": {"type":"entity","default":"tile","note":"e.g. Scene.<sid>.AmbientLight.<lid>"}, "collection": {"type":"string","values":["lights","sounds","tiles"],"default":"tiles"},
                "activate": {"type":"string","values":["activate","deactivate","toggle","previous"],"default":"deactivate"}}),
         json!({"entity":{"id":"Scene.SCENEID.AmbientLight.LIGHTID"},"collection":"lights","activate":"toggle"})),
        ("teleport", "Teleport the token(s) to a location (another tile, a point, another scene).",
         json!({"entity": {"type":"entity","default":"token"}, "location": {"type":"location","required":true},
                "position": {"type":"string","values":["random","center","relative"]}, "remotesnap": {"type":"boolean"},
                "animatepan": {"type":"boolean"}, "triggerremote": {"type":"boolean","default":false,"note":"fire the destination tile"},
                "deletesource": {"type":"boolean"}, "preservesettings": {"type":"boolean"}, "avoidtokens": {"type":"boolean"}, "colour": {"type":"string","note":"#rrggbb"}}),
         json!({"entity":{"id":"token"},"location":{"id":"Scene.SCENEID.Tile.TILEID"},"position":"center"})),
        ("movetoken", "Move (animate) the token(s) to a location.",
         json!({"entity": {"type":"entity","default":"token"}, "location": {"type":"location","required":true},
                "position": {"type":"string","values":["random","center","relative"]}, "snap": {"type":"boolean","default":true},
                "speed": {"type":"number","default":6}, "duration": {"type":"number"}, "trigger": {"type":"boolean","note":"fire tiles on the way"}}),
         json!({"entity":{"id":"token"},"location":{"x":1000,"y":600,"sceneId":"SCENEID"},"snap":true})),
        ("playsound", "Play an audio file.",
         json!({"audiofile": {"type":"string","required":true,"note":"path; wildcards allowed"}, "audiofor": {"type":"recipients","default":"trigger"},
                "volume": {"type":"string","default":"1.0"}, "loop": {"type":"boolean"}, "fade": {"type":"number","default":0.25},
                "scenerestrict": {"type":"boolean"}, "prevent": {"type":"boolean","note":"don't restart if already playing"}}),
         json!({"audiofile":"sounds/notify.wav","audiofor":"everyone","volume":"0.8"})),
        ("stopsound", "Stop sounds started by tiles.",
         json!({"audiotype": {"type":"string","values":["all","tile"]}, "entity": {"type":"entity"}, "audiofor": {"type":"recipients"}, "fade": {"type":"number","default":0.25}}),
         json!({"audiotype":"all","audiofor":"everyone"})),
        ("playlist", "Play/stop a playlist or a playlist sound.",
         json!({"entity": {"type":"entity","note":"Playlist.<id> or Playlist.<id>.PlaylistSound.<sid>"}, "play": {"type":"string","values":["play","stop","pause"]},
                "volume": {"type":"number"}, "loop": {"type":"boolean"}}),
         json!({"entity":{"id":"Playlist.PLAYLISTID"},"play":"play"})),
        ("changedoor", "Change a door wall.",
         json!({"entity": {"type":"entity","required":true,"note":"Scene.<sid>.Wall.<wid>"},
                "type": {"type":"string","note":"nothing | toggle | NONE | DOOR | SECRET"},
                "state": {"type":"string","note":"nothing | toggle | CLOSED | OPEN | LOCKED"},
                "movement": {"type":"string"}, "light": {"type":"string"}, "sight": {"type":"string"}, "sound": {"type":"string"}}),
         json!({"entity":{"id":"Scene.SCENEID.Wall.WALLID"},"state":"OPEN","type":"nothing"})),
        ("rolltable", "Roll on a table.",
         json!({"rolltableid": {"type":"entity","required":true,"note":"{id:\"RollTable.<id>\"} or a UUID"}, "quantity": {"type":"string","default":"1"},
                "rollmode": {"type":"string","values":["roll","gmroll","blindroll","selfroll"]}, "chatmessage": {"type":"boolean","default":true},
                "reset": {"type":"boolean","default":true}}),
         json!({"rolltableid":{"id":"RollTable.TABLEID"},"quantity":"1","chatmessage":true})),
        ("scene", "Switch scenes (for the triggering user or everyone).",
         json!({"sceneid": {"type":"entity","required":true,"note":"{id:\"Scene.<id>\"}"}, "activate": {"type":"boolean","default":false},
                "for": {"type":"recipients","default":"trigger"}}),
         json!({"sceneid":{"id":"Scene.SCENEID"},"activate":false,"for":"everyone"})),
        ("ping", "Ping a location on the canvas.",
         json!({"location": {"type":"location","required":true}, "style": {"type":"string"}, "pingfrom": {"type":"recipients","default":"trigger"}}),
         json!({"location":{"id":"tile"}})),
        ("pancanvas", "Pan the camera.",
         json!({"location": {"type":"location","required":true}, "animate": {"type":"boolean"}, "duration": {"type":"number","default":1},
                "panfor": {"type":"recipients","default":"trigger"}}),
         json!({"location":{"id":"tile"},"animate":true,"panfor":"everyone"})),
        ("scrollingtext", "Floating text above a token or tile.",
         json!({"text": {"type":"string","required":true}, "entity": {"type":"entity"}, "for": {"type":"recipients","default":"trigger"},
                "duration": {"type":"number","default":5}, "anchor": {"type":"number","default":0}, "direction": {"type":"number","default":2}}),
         json!({"text":"Click!","entity":{"id":"token"},"for":"everyone"})),
        ("scenelighting", "Change the scene darkness.",
         json!({"darkness": {"type":"number","default":1,"note":"0 (light) – 1 (dark)"}, "speed": {"type":"number","default":10}}),
         json!({"darkness":0.8,"speed":5})),
        ("trigger", "Fire another active tile.",
         json!({"entity": {"type":"entity","required":true,"note":"Scene.<sid>.Tile.<tid>"}, "token": {"type":"entity"},
                "landing": {"type":"string","note":"anchor tag to start at"}, "return": {"type":"boolean","default":true}, "allowdisabled": {"type":"boolean","default":false}}),
         json!({"entity":{"id":"Scene.SCENEID.Tile.TILEID"}})),
        ("resethistory", "Clear a tile's trigger history.",
         json!({"entity": {"type":"entity","default":"tile"}, "resettype": {"type":"string","default":"all"}, "token": {"type":"entity"}}),
         json!({"entity":{"id":"tile"}})),
        ("setvariable", "Store a tile variable (read back by checkvariable).",
         json!({"entity": {"type":"entity","default":"tile"}, "name": {"type":"string","required":true}, "value": {"type":"value","required":true}}),
         json!({"entity":{"id":"tile"},"name":"opened","value":"1"})),
        ("anchor", "Label a point in the list (goto target). Tags starting with « _ » start the list for a trigger: « _enter », « _exit », « _click », « _gm »…",
         json!({"tag": {"type":"string","required":true}, "stop": {"type":"boolean","default":true,"note":"stop when reached in sequence"}}),
         json!({"tag":"_exit","stop":true})),
        ("goto", "Jump to an anchor.",
         json!({"tag": {"type":"string","required":true}, "limit": {"type":"number","note":"max number of jumps"}}),
         json!({"tag":"again","limit":3})),
        ("stop", "Stop the remaining actions.",
         json!({"entity": {"type":"entity","default":"tile"}}),
         json!({"entity":{"id":"tile"}})),
        ("stoptriggers", "Stop other tiles from firing on this movement.", json!({}), json!({})),
        ("checkvalue", "Continue only if a value matches.",
         json!({"name": {"type":"string","required":true}, "value": {"type":"value","required":true,"note":"« > 3 », « = 1 »"}, "fail": {"type":"string","note":"anchor on failure"}}),
         json!({"name":"opened","value":"= 1","fail":"closed"})),
        ("exists", "Continue only if documents exist (e.g. tokens within the tile).",
         json!({"entity": {"type":"entity"}, "collection": {"type":"string"}, "count": {"type":"value","default":"> 0"}, "none": {"type":"string","note":"anchor when none"}}),
         json!({"entity":{"id":"within"},"collection":"tokens","count":"> 1"})),
        ("triggercount", "Continue only after N triggers (needs record).",
         json!({"count": {"type":"value","default":"> 1"}, "unique": {"type":"boolean"}, "none": {"type":"string"}}),
         json!({"count":"> 2"})),
        ("random", "Continue with a percentage chance.",
         json!({"number": {"type":"number","note":"percent"}, "fail": {"type":"string","note":"anchor on failure"}}),
         json!({"number":50})),
        ("openjournal", "Open a journal entry for users.",
         json!({"entity": {"type":"entity","required":true,"note":"JournalEntry.<id>"}, "page": {"type":"string"},
                "showto": {"type":"recipients","default":"everyone"}, "permission": {"type":"string","default":"false"}}),
         json!({"entity":{"id":"JournalEntry.JOURNALID"},"showto":"everyone"})),
    ]
}

#[rustfmt::skip]
fn refused() -> Value {
    json!([
        {"what": "runcode", "why": "runs arbitrary JavaScript in the GM's browser"},
        {"what": "runbatch", "why": "runs code in batches"},
        {"what": "url", "why": "opens arbitrary URLs / javascript: links"},
        {"what": "dialog", "why": "its buttons and content execute code"},
        {"what": "actions from other modules / unknown actions", "why": "can't be checked, may run code"},
        {"what": "runmacro", "why": "runs a macro: needs the companion setting « Remote script-macro execution » = auto (use client_run_macro instead, the GM confirms)"},
        {"what": "value fields that are not number / operator+number / [[dice]] / true|false / identifier", "why": "MAT evaluates them with eval(): « = alert(1) », « = \"text\" », « = token.name » are code"},
        {"what": "« {{ » anywhere", "why": "texts are compiled by Handlebars with prototype access — that's code ({{constructor}}…)"},
        {"what": "texts starting with « = », « + », « - »", "why": "MAT evaluates such texts as expressions"},
        {"what": "active HTML (<script>, <iframe>, on…= handlers, javascript:)", "why": "rendered in chat/notifications"},
        {"what": "partial writes of the action list (flags.monks-active-tiles.actions.N…)", "why": "can't be validated alone — mat_update_tile always re-writes the whole list"},
        {"what": "Region behaviors executeScript / executeMacro", "why": "same as runcode/runmacro, through core Regions"},
    ])
}

fn action_types(args: &Value) -> Result<Value> {
    let catalog = action_catalog();
    let entry = |(k, d, f, e): &(&str, &str, Value, Value)| json!({"action": k, "description": d, "fields": f, "example": {"action": k, "data": e}});
    if let Some(a) = str_arg(args, "action") {
        if let Some(c) = catalog.iter().find(|c| c.0 == a) {
            return Ok(text_response(
                &json!({"action": entry(c), "references": references()}),
            ));
        }
        if crate::foundry::script_guard::MAT_SAFE_ACTIONS.contains(&a.as_str()) {
            return Ok(text_response(&json!({
                "action": a, "safe": true,
                "note": "allowed by the guard but not detailed here — its data fields follow MAT's action config; values must still pass the value rules",
                "references": references(),
            })));
        }
        bail!(
            "« {a} » is not a safe action: {} Documented actions: {}",
            refused()
                .as_array()
                .and_then(|r| r.iter().find(|x| x["what"].as_str() == Some(a.as_str())))
                .and_then(|x| x["why"].as_str())
                .map(|w| format!("{w}."))
                .unwrap_or_else(|| "it is unknown or runs code.".into()),
            catalog.iter().map(|c| c.0).collect::<Vec<_>>().join(", ")
        );
    }
    let documented: Vec<&str> = catalog.iter().map(|c| c.0).collect();
    let other_safe: Vec<&str> = crate::foundry::script_guard::MAT_SAFE_ACTIONS
        .iter()
        .copied()
        .filter(|a| !documented.contains(a))
        .collect();
    Ok(text_response(&json!({
        "tile_shape": "flags[\"monks-active-tiles\"] = {active, trigger:[…], restriction, controlled, chance, pertoken, record, minrequired, cooldown, actions:[{id, action, data}]} — actions run in order",
        "triggers": TRIGGERS.iter().map(|(k, d)| json!({"trigger": k, "meaning": d})).collect::<Vec<_>>(),
        "legacy_triggers": {"both": "enter + exit", "hover": "hoverin + hoverout"},
        "conditions": {
            "restriction": "all | player | gm — whose tokens", "controlled": "all | player | gm — which users",
            "chance": "0-100, movement triggers only", "once_per_token": "pertoken + record",
            "cooldown": "seconds", "minrequired": "fire after N recorded triggers",
        },
        "references": references(),
        "actions": catalog.iter().map(entry).collect::<Vec<_>>(),
        "other_safe_actions": other_safe,
        "refused_by_the_gateway": refused(),
        "notes": [
            "Hidden tiles still fire (invisible traps).",
            "Movement triggers (enter/exit/…) do nothing with an empty action list.",
            "Movement triggers fire for moves made in a client (drag by a player/the GM), not for server-side moves (move_token / update_token): fire tiles from the gateway with client_mat_trigger.",
            "Tiles only run in a connected GM's browser with the scene's canvas loaded.",
            "hurtheal uses the system's primary token attribute — wrong for starwarsffg; use alter on system.stats.wounds.value.",
        ],
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declencheurs_valides_et_inconnus() {
        assert_eq!(parse_triggers(&json!("enter")).unwrap(), vec!["enter"]);
        assert_eq!(
            parse_triggers(&json!(["both", "enter", "click"])).unwrap(),
            vec!["enter", "exit", "click"]
        );
        let e = parse_triggers(&json!(["enter", "teleport"]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("teleport") && e.contains("manual"));
        assert!(parse_triggers(&json!([])).is_err());
        assert!(parse_triggers(&json!(3)).is_err());
    }

    #[test]
    fn actions_ids_generes_et_uniques() {
        let a = normalize_actions(
            &json!([{"action":"pause","data":{"pause":"pause"}},{"action":"delay","id":"abc123"}]),
            &[],
        )
        .unwrap();
        let id0 = a[0]["id"].as_str().unwrap();
        assert_eq!(id0.len(), 16);
        assert!(id0.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(a[1]["id"], "abc123");
        assert_eq!(a[1]["data"], json!({}));
        assert!(
            normalize_actions(
                &json!([{"action":"pause","id":"x"},{"action":"pause","id":"x"}]),
                &[]
            )
            .is_err()
        );
        assert!(normalize_actions(&json!([{"action":"pause","id":"x"}]), &["x".into()]).is_err());
        assert!(normalize_actions(&json!([{"data":{}}]), &[]).is_err());
        assert!(normalize_actions(&json!([{"action":"pause","id":"a b"}]), &[]).is_err());
        assert!(normalize_actions(&json!({"action":"pause"}), &[]).is_err());
    }

    #[test]
    fn options_et_once_per_token() {
        let f = option_flags(&json!({"trigger":"manual","once_per_token":true,"chance":50,"restriction":"player","minrequired":2})).unwrap();
        assert_eq!(f["trigger"], json!(["manual"]));
        assert_eq!(f["pertoken"], json!(true));
        assert_eq!(f["record"], json!(true));
        assert_eq!(f["minrequired"], json!(2));
        assert!(option_flags(&json!({"restriction":"everyone"})).is_err());
        assert!(option_flags(&json!({"chance":150})).is_err());
        assert!(option_flags(&json!({"minrequired":1.5})).is_err());
        let f = option_flags(&json!({"once_per_token":false})).unwrap();
        assert_eq!(f["pertoken"], json!(false));
        assert!(!f.contains_key("record"));
    }

    #[test]
    fn geometrie_pixels_et_cases() {
        let scene = json!({"grid":{"size":50}});
        assert_eq!(grid_size(&scene), 50.0);
        assert_eq!(grid_size(&json!({"grid":140})), 140.0);
        assert_eq!(grid_size(&json!({})), 100.0);
        let g = geometry(&json!({"grid":{"col":2,"row":3,"w":2}}), 50.0).unwrap();
        assert_eq!(
            (
                g["x"].clone(),
                g["y"].clone(),
                g["width"].clone(),
                g["height"].clone()
            ),
            (json!(100.0), json!(150.0), json!(100.0), json!(50.0))
        );
        let g = geometry(&json!({"x":10,"y":20}), 50.0).unwrap();
        assert_eq!(g.len(), 2);
        assert!(geometry(&json!({"grid":{"col":1,"row":1},"x":5}), 50.0).is_err());
        assert!(geometry(&json!({"grid":{"col":1}}), 50.0).is_err());
        assert!(geometry(&json!({"width":0}), 50.0).is_err());
    }

    #[test]
    fn recherche_de_tuile() {
        let scene = json!({"_id":"S","name":"Scène","tiles":[
            {"_id":"a","name":"Piège"},
            {"_id":"b","flags":{"monks-active-tiles":{"name":"Bouton"}}},
            {"_id":"c","name":"Double"},{"_id":"d","name":"Double"}]});
        assert_eq!(find_tile(&scene, "a").unwrap()["_id"], "a");
        assert_eq!(find_tile(&scene, "Piège").unwrap()["_id"], "a");
        assert_eq!(find_tile(&scene, "Bouton").unwrap()["_id"], "b");
        assert!(
            find_tile(&scene, "Double")
                .unwrap_err()
                .to_string()
                .contains("c, d")
        );
        assert!(find_tile(&scene, "Nope").is_err());
        assert_eq!(tile_uuid(&scene, &json!({"_id":"a"})), "Scene.S.Tile.a");
    }

    #[test]
    fn vue_et_historique() {
        let scene = json!({"_id":"S","name":"Sc","tokens":[{"_id":"t1","name":"Kira"}]});
        let tile = json!({"_id":"a","x":0,"y":0,"width":100,"height":100,"hidden":true,"texture":{"src":"i.svg"},
            "flags":{"monks-active-tiles":{"active":true,"trigger":"enter","pertoken":true,"record":true,
              "actions":[{"id":"x","action":"runcode","data":{"code":"1"}}],
              "history":{"t1":{"tokenid":"t1","triggered":[{"id":"h","who":"u","how":"enter","when":1},{"id":"h2","who":"u","how":"enter","when":2}]},
                         "_blank":{"tokenid":"_blank","triggered":[{"id":"h3","who":"u","how":"manual","when":3}]}}}}});
        let v = tile_view(&scene, &tile);
        assert_eq!(v["trigger"], json!(["enter"]));
        assert_eq!(v["conditions"]["once_per_token"], json!(true));
        assert_eq!(v["history"]["total"], json!(3));
        let k = v["history"]["by_token"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["token"] == "t1")
            .unwrap();
        assert_eq!(k["name"], "Kira");
        assert_eq!(k["count"], 2);
        assert_eq!(v["companion_would_refuse"]["needs"], "runScriptMode=auto");
    }

    #[test]
    fn messages_du_garde_fou_explicites() {
        let n = check_mat_actions(
            &json!([{"id":"a","action":"alter","data":{"attribute":"x","value":"= alert(1)"}}]),
        )
        .unwrap();
        let m = guard_error(&n, "off", "confirm");
        assert!(
            m.contains("alter")
                && m.contains("value")
                && m.contains("« off »")
                && m.contains("operator and a number")
        );
        let n =
            check_mat_actions(&json!([{"id":"a","action":"runcode","data":{"code":"x"}}])).unwrap();
        assert!(guard_error(&n, "off", "confirm").contains("client_run_macro"));
        let n = check_mat_actions(
            &json!([{"id":"a","action":"chatmessage","data":{"text":"{{constructor}}"}}]),
        )
        .unwrap();
        assert!(guard_error(&n, "off", "confirm").contains("Handlebars"));
        let n = check_mat_actions(&json!([{"id":"a","action":"runmacro","data":{}}])).unwrap();
        assert!(guard_error(&n, "off", "confirm").contains("script-macro"));
    }

    #[test]
    fn catalogue_conforme_au_garde_fou() {
        // Chaque action documentée est sûre et son exemple passe le garde-fou.
        for (k, _, _, ex) in action_catalog() {
            assert!(
                crate::foundry::script_guard::MAT_SAFE_ACTIONS.contains(&k),
                "{k}"
            );
            let list = json!([{"id":"a","action":k,"data":ex}]);
            assert!(check_mat_actions(&list).is_none(), "exemple de {k} refusé");
        }
        assert!(action_types(&json!({"action":"chatmessage"})).is_ok());
        assert!(action_types(&json!({"action":"elevation"})).is_ok());
        assert!(
            action_types(&json!({"action":"runcode"}))
                .unwrap_err()
                .to_string()
                .contains("JavaScript")
        );
        assert!(action_types(&json!({})).is_ok());
        for d in definitions() {
            assert!(d.1.contains("Monk's Active Tiles"));
        }
    }
}
