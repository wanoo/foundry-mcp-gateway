//! Item Piles — piles, coffres, marchands, coffres-forts (ip_*)
//!
//! Lecture côté serveur (ip_status, ip_list_piles, ip_get_pile) : tout est
//! dans les documents — flags `flags.item-piles.data` (stockés CLAIRSEMÉS :
//! seules les clés ≠ défauts), objets embarqués, réglages monde `item-piles.*`.
//! Écriture par le compagnon (game.itempiles.API) : création de token, piles
//! d'objets, monnaies, transformation de tokens et regarnissage passent par la
//! logique du module (empilement, transformateurs système, hooks), que la
//! passerelle ne peut pas reproduire fidèlement.
//!
//! Depuis la 3.x, le cœur d'Item Piles ne connaît AUCUN système : chaque
//! système a besoin d'un module d'intégration (dnd5e : `itempilesdnd5e`) ou
//! d'une configuration manuelle. Sans monnaies ni chemin de quantité, tous les
//! outils client_ip_* refusent avec le diagnostic d'ip_status.

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::foundry::client::FoundryHandle;
use crate::foundry::documents::get_path;
use crate::mcp::McpState;
use crate::tools::ToolDef;
use crate::tools::companion::{Delivery, call_companion, cmd_for};
use crate::tools::{carry, str_arg, text_response};

const IP_ID: &str = "item-piles";

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

pub const COMMANDS: &[(&str, &str, Delivery)] = &[
    ("client_ip_create_pile", "ip_create_pile", Delivery::Unique),
    ("client_ip_add_items", "ip_add_items", Delivery::Unique),
    (
        "client_ip_remove_items",
        "ip_remove_items",
        Delivery::Unique,
    ),
    (
        "client_ip_configure_pile",
        "ip_configure_pile",
        Delivery::Unique,
    ),
    (
        "client_ip_turn_into_pile",
        "ip_turn_into_pile",
        Delivery::Unique,
    ),
    ("client_ip_revert_pile", "ip_revert_pile", Delivery::Unique),
    ("client_ip_restock", "ip_restock", Delivery::Unique),
];

pub const READ_ONLY: &[&str] = &["ip_status", "ip_list_piles", "ip_get_pile"];

/// Clés de configuration de pile acceptées en écriture (noms Item Piles).
/// `macro` en est absente À DESSEIN : une macro de pile s'exécute chez le MJ à
/// chaque interaction — le garde-fou de la passerelle la refuse, le compagnon
/// aussi ; seul le MJ la pose, dans Foundry.
#[rustfmt::skip]
pub const CONFIG_KEYS: &[&str] = &[
    "type", "distance", "deleteWhenEmpty", "canStackItems", "canInspectItems",
    "displayItemTypes", "description", "overrideItemFilters", "overrideCurrencies",
    "overrideSecondaryCurrencies", "requiredItemProperties",
    "displayOne", "showItemName", "overrideSingleItemScale", "singleItemScale",
    "shareItemsEnabled", "shareCurrenciesEnabled", "takeAllEnabled", "splitAllEnabled", "activePlayers",
    "closed", "locked", "closedImage", "emptyImage", "openedImage", "lockedImage",
    "closeSound", "openSound", "lockedSound", "unlockedSound",
    "merchantImage", "infiniteQuantity", "infiniteCurrencies", "purchaseOnly", "hideNewItems",
    "hideItemsWithZeroCost", "keepZeroQuantity", "onlyAcceptBasePrice", "displayQuantity",
    "buyPriceModifier", "sellPriceModifier", "itemTypePriceModifiers", "tablesForPopulate",
    "hideTokenWhenClosed", "openTimes", "closedDays", "refreshItemsOnOpen", "logMerchantActivity",
    "cols", "rows", "restrictVaultAccess", "vaultExpansion", "baseExpansionCols", "baseExpansionRows",
    "logVaultActions",
];

pub const PILE_TYPES: &[&str] = &["pile", "container", "merchant", "vault"];

/// Modules d'intégration connus, par système.
#[rustfmt::skip]
const SYSTEM_ADDONS: &[(&str, &str)] = &[
    ("dnd5e", "install the « Item Piles: D&D 5e » module (id itempilesdnd5e)"),
    ("starwarsffg", "install the Star Wars FFG integration for Item Piles (github.com/maersk-aerospace/FoundryVTT-ItemPiles-StarWarsFFG)"),
    ("pf2e", "install the « Item Piles: PF2e » integration module"),
];

#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    let target = json!({"type":"string","description":"The pile: an Actor UUID (\"Actor.abc\"), a token UUID (\"Scene.s.Token.t\" — use it for unlinked tokens, whose contents live on the token), or a world actor id/name."});
    let items = json!({"type":"array","description":"Items to add, copied from world items or compendiums. Quantities stack with matching items already in the pile (Item Piles similarity rules).",
        "items":{"type":"object","properties":{
            "uuid":{"type":"string","description":"Item UUID: \"Item.abc\" or \"Compendium.dnd5e.items.Item.xyz\"."},
            "quantity":{"type":"integer","minimum":1,"description":"Default 1."},
            "hidden":{"type":"boolean","description":"Hide the item from players (merchants)."},
            "not_for_sale":{"type":"boolean"},
            "infinite_quantity":{"type":"boolean","description":"Merchant: never runs out."},
            "free":{"type":"boolean"}},
            "required":["uuid"]}});
    let currencies = json!({"description":"Currencies, as an Item Piles string using the configured abbreviations (\"50gp 3sp\", dice allowed: \"2d6gp\") or an object {\"gp\":50,\"sp\":3}. Unknown abbreviations are rejected (ip_status lists them).",
        "oneOf":[{"type":"string"},{"type":"object","additionalProperties":{"type":"number"}}]});
    let config = json!({"type":"object","description":"Item Piles settings to apply (Item Piles key names), e.g. {closed:true, locked:true, deleteWhenEmpty:false, shareItemsEnabled:true, buyPriceModifier:1.2, sellPriceModifier:0.4, infiniteQuantity:true, purchaseOnly:true, hideNewItems:true, displayOne:true, cols:8, rows:4}. tablesForPopulate accepts [{uuid, timesToRoll:\"1d4\", addAll:false}]. Allowed keys: type, distance, deleteWhenEmpty, canStackItems, canInspectItems, displayItemTypes, description, overrideItemFilters, overrideCurrencies, requiredItemProperties, displayOne, showItemName, overrideSingleItemScale, singleItemScale, shareItemsEnabled, shareCurrenciesEnabled, takeAllEnabled, splitAllEnabled, activePlayers, closed, locked, closedImage, emptyImage, openedImage, lockedImage, closeSound, openSound, lockedSound, unlockedSound, merchantImage, infiniteQuantity, infiniteCurrencies, purchaseOnly, hideNewItems, hideItemsWithZeroCost, keepZeroQuantity, onlyAcceptBasePrice, displayQuantity, buyPriceModifier, sellPriceModifier, itemTypePriceModifiers, tablesForPopulate, hideTokenWhenClosed, openTimes, closedDays, refreshItemsOnOpen, logMerchantActivity, cols, rows, restrictVaultAccess, vaultExpansion, baseExpansionCols, baseExpansionRows, logVaultActions. `macro` is refused: a pile macro runs in the GM's browser on every interaction, so only the GM may attach one, in Foundry."});
    let pile_type = json!({"type":"string","enum": PILE_TYPES});
    vec![
        ("ip_status",
         "Item Piles: is the module active and configured for this game system? Reports the system, the currencies (name, abbreviation, attribute path or item), secondary currencies, the item quantity and price attribute paths, the actor type used for pile actors, the item filters, and the detected system integration module. Item Piles 3.x supports NO system by itself: each one needs an integration module (dnd5e: itempilesdnd5e; starwarsffg: maersk-aerospace/FoundryVTT-ItemPiles-StarWarsFFG) or a manual setup in Item Piles' module settings. When `ready` is false, every client_ip_* tool refuses with the same `message`.",
         json!({"type":"object","properties":{}})),
        ("ip_list_piles",
         "Item Piles: list item piles, read from the documents (no companion needed). scope world = actors flagged as piles (flags.item-piles.data.enabled), scene = tokens of a scene that are piles, all (default) = both. Each entry: uuid (pass it to ip_get_pile and client_ip_*), name, pile type (pile/container/merchant/vault), closed/locked, and for tokens the scene, position, linked flag and actor. type filters on the pile type.",
         json!({"type":"object","properties":{
            "scope":{"type":"string","enum":["world","scene","all"],"description":"Default all."},
            "scene":{"type":"string","description":"Scene id or name for the token listing. Default: the active scene."},
            "type": pile_type.clone()}})),
        ("ip_get_pile",
         "Item Piles: read a pile, container, merchant or vault server-side — its effective settings (Item Piles defaults ← world pileDefaults ← the pile's own flags: type, closed, locked, deleteWhenEmpty, sharing; merchant: buy/sell price modifiers, per-type and per-actor modifiers, infiniteQuantity/infiniteCurrencies, purchaseOnly, population tables; vault: grid size), its items (id, name, type, quantity and price read at the configured attribute paths, hidden/notForSale/infiniteQuantity/free flags) and its currencies (configured list, attribute or item based). Items excluded by Item Piles' item filters (spells, features… in dnd5e) are counted apart. For an unlinked token give the token UUID: its contents are the base actor's merged with the token's delta. hasMacro tells whether a pile macro is attached (writes through client_ip_* then require the GM's macro permission).",
         json!({"type":"object","properties":{"uuid": target.clone()},"required":["uuid"]})),
        ("client_ip_create_pile",
         "Item Piles (companion): create a prep pile through game.itempiles.API.createItemPile — a NEW dedicated actor named `name` (never Item Piles' shared « Default Item Pile », whose deleteWhenEmpty would delete the token once looted), plus a token at x,y (scene pixels) on scene_id or, by default, the scene the GM is viewing. Without x/y only the actor is created (drag it onto a scene later). type: pile (loot on the ground), container (chest: can be closed/locked), merchant (shop with buy/sell prices), vault (grid storage). linked (default true) links the token to the actor so contents live on the actor; false makes an unlinked token whose contents live on the token. deleteWhenEmpty defaults to false (prep piles survive being emptied). items + currencies are added after creation through the API (stacking applied). Returns actorUuid, tokenUuid and the pile's actual contents read back. Refused when Item Piles is not configured for the system (see ip_status). Every API call that Item Piles reports as failed (returns false: a pre-hook cancelled it or no GM executed it) becomes an error.",
         json!({"type":"object","properties":{
            "type": pile_type.clone(),
            "name":{"type":"string","description":"Actor and token name."},
            "x":{"type":"number"},"y":{"type":"number"},
            "scene_id":{"type":"string","description":"Scene id. Default: the scene the GM is viewing."},
            "img":{"type":"string","description":"Actor and token image path."},
            "linked":{"type":"boolean","description":"Default true."},
            "folder":{"type":"string","description":"Actor folder name (created if missing)."},
            "items": items.clone(), "currencies": currencies.clone(), "config": config.clone()},
            "required":["type","name"]})),
        ("client_ip_add_items",
         "Item Piles (companion): add items (copied by UUID from world items or compendiums, stacked with matching items) and/or currencies to an existing pile, through addItems / addCurrencies. Returns what was added and the pile's contents read back. If the pile has a macro attached, the companion asks for the GM's « Remote script-macro execution » permission first (Item Piles runs it on every change).",
         json!({"type":"object","properties":{"uuid": target.clone(),"items": items.clone(),"currencies": currencies.clone()},"required":["uuid"]})),
        ("client_ip_remove_items",
         "Item Piles (companion): remove items and/or currencies from a pile, through removeItems / removeCurrencies. Items are designated by their id ON THE PILE (from ip_get_pile) or by exact name; quantity omitted = the whole stack. Removing more currency than the pile holds is an error; change:true (default) lets Item Piles break larger coins (10gp from 1pp leaves change), false refuses when change would be needed. Returns the pile's contents read back.",
         json!({"type":"object","properties":{
            "uuid": target.clone(),
            "items":{"type":"array","items":{"type":"object","properties":{
                "id":{"type":"string"},"name":{"type":"string"},"quantity":{"type":"integer","minimum":1}}}},
            "currencies": currencies.clone(),
            "change":{"type":"boolean","description":"Default true."}},"required":["uuid"]})),
        ("client_ip_configure_pile",
         "Item Piles (companion): change a pile's settings through updateItemPile (deep-merged into its flags; on a token it also updates the token). config takes a whitelist of Item Piles keys (see the parameter); `macro` is refused, as the gateway's own write guard refuses flags.item-piles.data.macro — a pile macro is code run in the GM's browser, the GM attaches it in Foundry. price_modifiers sets per-actor merchant modifiers through updateMerchantPriceModifiers ({actor_uuid, buy, sell, relative, override}). Toggling closed/locked runs the pile's macro if it has one, so that case asks for the GM's macro permission. Returns the effective settings read back.",
         json!({"type":"object","properties":{
            "uuid": target.clone(), "config": config.clone(),
            "price_modifiers":{"type":"array","items":{"type":"object","properties":{
                "actor_uuid":{"type":"string"},"buy":{"type":"number"},"sell":{"type":"number"},
                "relative":{"type":"boolean","description":"Add to the current modifier instead of replacing it."},
                "override":{"type":"boolean","description":"Ignore the merchant's other modifiers for this actor."}},"required":["actor_uuid"]}}},
            "required":["uuid"]})),
        ("client_ip_turn_into_pile",
         "Item Piles (companion): turn existing tokens into piles (turnTokensIntoItemPiles) — e.g. a defeated monster becomes lootable, a placed chest token becomes a container. token_ids on scene_id (default: the scene the GM is viewing). type + config set the pile settings (same whitelist as client_ip_configure_pile, deleteWhenEmpty defaults to false). Linked tokens turn their actor into a pile too. Settings of Item Piles' « Default Item Pile » actor, if it exists, are merged in by the module. Returns the token UUIDs.",
         json!({"type":"object","properties":{
            "token_ids":{"type":"array","items":{"type":"string"},"minItems":1},
            "scene_id":{"type":"string"},
            "type": pile_type.clone(), "config": config.clone()},"required":["token_ids"]})),
        ("client_ip_revert_pile",
         "Item Piles (companion): turn piles back into ordinary tokens (revertTokensFromItemPiles: the pile settings are kept but disabled, items stay on the actor). token_ids on scene_id (default: the scene the GM is viewing). Returns the token UUIDs.",
         json!({"type":"object","properties":{
            "token_ids":{"type":"array","items":{"type":"string"},"minItems":1},
            "scene_id":{"type":"string"}},"required":["token_ids"]})),
        ("client_ip_restock",
         "Item Piles (companion): fill a pile from roll tables. Without `table`: refresh a merchant from its own population tables (tablesForPopulate, set with client_ip_configure_pile) through refreshMerchantInventory. With `table` (RollTable UUID, id or name — world or compendium): roll it times_to_roll times (number or dice formula, default 1) into any pile through rollItemTable. remove_existing clears the current items first (default true for a merchant refresh, false for a table roll). Table results that are not items are ignored. Returns the items added and the pile's contents read back.",
         json!({"type":"object","properties":{
            "uuid": target,
            "table":{"type":"string"},
            "times_to_roll":{"description":"Number or formula, e.g. \"1d4\". Default 1.","oneOf":[{"type":"string"},{"type":"integer"}]},
            "remove_existing":{"type":"boolean"}},"required":["uuid"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

// ------------------------------------------------------------- logique pure

/// Valeur d'un document Setting, décodée du JSON où Foundry la stocke.
fn decode_setting(v: &Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| v.clone()),
        other => other.clone(),
    }
}

fn non_empty(v: Option<&Value>) -> bool {
    match v {
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

/// Le diagnostic d'ip_status, à partir de l'id système, de
/// `core.moduleConfiguration` et des réglages `item-piles.*` (clés sans
/// préfixe). Pur : testé sans Foundry.
pub fn assess_status(system: &str, modules: &Map<String, Value>, ip: &Map<String, Value>) -> Value {
    let active = modules.get(IP_ID).and_then(Value::as_bool).unwrap_or(false);
    // Modules d'intégration actifs : « itempiles<système> », « item-piles-… ».
    let addons: Vec<&String> = modules
        .iter()
        .filter(|(id, on)| {
            let l = id.to_ascii_lowercase();
            on.as_bool() == Some(true)
                && l != IP_ID
                && (l.starts_with("itempiles")
                    || l.starts_with("item-piles-")
                    || l.contains("itempiles"))
        })
        .map(|(id, _)| id)
        .collect();
    let currencies = ip.get("currencies").cloned().unwrap_or(json!([]));
    let qty = ip
        .get("itemQuantityAttribute")
        .cloned()
        .unwrap_or(Value::Null);
    let configured = non_empty(Some(&currencies)) && non_empty(Some(&qty));
    let ready = active && configured;
    let hint = SYSTEM_ADDONS
        .iter()
        .find(|(s, _)| *s == system)
        .map(|(_, h)| h.to_string())
        .unwrap_or_else(|| {
            "no integration module is known for this system: configure currencies, the item \
             quantity and price attributes and the actor type by hand in Item Piles' module settings"
                .into()
        });
    let message = if !active {
        format!(
            "Item Piles (item-piles) is not active in this world. Activate it, then for « {system} »: {hint}."
        )
    } else if !configured {
        format!(
            "Item Piles is active but not configured for the « {system} » system (no currencies or no item quantity attribute — Item Piles 3.x supports no system on its own). {}{hint}, reload the GM's browser, then check ip_status again.",
            if addons.is_empty() {
                String::new()
            } else {
                format!(
                    "Integration module(s) {} active but they have not configured it yet; otherwise ",
                    addons
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        )
    } else {
        "Item Piles is active and configured.".to_string()
    };
    let present_currency = |c: &Value| {
        json!({
            "name": c.get("name"),
            "abbreviation": c.get("abbreviation"),
            "type": c.get("type").cloned().unwrap_or(json!("attribute")),
            "path": c.pointer("/data/path"),
            "itemUuid": c.pointer("/data/uuid"),
            "primary": c.get("primary").cloned().unwrap_or(json!(false)),
            "exchangeRate": c.get("exchangeRate"),
        })
    };
    let list = |k: &str| -> Vec<Value> {
        ip.get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().map(present_currency).collect())
            .unwrap_or_default()
    };
    json!({
        "ready": ready,
        "message": message,
        "system": system,
        "moduleActive": active,
        "configured": configured,
        "integrationModules": addons,
        "suggestedIntegration": if configured { Value::Null } else { json!(hint) },
        "systemVersion": ip.get("systemVersion"),
        "currencies": list("currencies"),
        "secondaryCurrencies": list("secondaryCurrencies"),
        "itemQuantityAttribute": qty,
        "itemPriceAttribute": ip.get("itemPriceAttribute"),
        "quantityForPriceAttribute": ip.get("quantityForPriceAttribute"),
        "actorClassType": ip.get("actorClassType"),
        "itemFilters": ip.get("itemFilters"),
        "deleteEmptyPiles": ip.get("deleteEmptyPiles"),
    })
}

/// Valide un objet `config` : clés de la liste blanche uniquement, `macro`
/// refusée avec la raison, type de pile connu, nombres positifs pour les
/// modificateurs. Renvoie l'objet tel quel.
pub fn validate_config(config: &Value) -> Result<Value> {
    let Some(obj) = config.as_object() else {
        bail!("config must be an object of Item Piles settings");
    };
    for (k, v) in obj {
        if k == "macro" {
            bail!(
                "config.macro is refused: an Item Piles macro runs in the GM's browser on every interaction with the pile. \
                 The gateway's write guard refuses flags.item-piles.data.macro unless the GM set the companion's « Remote script-macro execution » to auto; \
                 attach the macro in Foundry (pile configuration) instead"
            );
        }
        if k == "enabled" {
            bail!("config.enabled is managed by the tool (client_ip_revert_pile disables a pile)");
        }
        if !CONFIG_KEYS.contains(&k.as_str()) {
            bail!(
                "config.{k} is not a supported Item Piles setting (see the tool description for the allowed keys)"
            );
        }
        match k.as_str() {
            "type" if !v.as_str().is_some_and(|t| PILE_TYPES.contains(&t)) => {
                bail!("config.type must be one of {}", PILE_TYPES.join(", "))
            }
            "buyPriceModifier" | "sellPriceModifier" | "singleItemScale" | "distance"
                if !v.as_f64().is_some_and(|n| n >= 0.0) =>
            {
                bail!("config.{k} must be a number ≥ 0")
            }
            "cols" | "rows" | "baseExpansionCols" | "baseExpansionRows"
                if !v.as_i64().is_some_and(|n| n >= 0) =>
            {
                bail!("config.{k} must be an integer ≥ 0")
            }
            _ => {}
        }
    }
    Ok(config.clone())
}

/// Défauts du cœur d'Item Piles 3.3.4 (constants.js, PILE_DEFAULTS) — les
/// clés utiles à la lecture ; les flags n'en stockent que les écarts.
fn core_defaults() -> Value {
    serde_json::from_str(CORE_DEFAULTS).expect("CORE_DEFAULTS is valid JSON")
}

const CORE_DEFAULTS: &str = r#"{
    "enabled": false, "type": "pile", "distance": 1, "macro": "", "deleteWhenEmpty": "default",
    "canStackItems": "yes", "canInspectItems": true, "displayItemTypes": false, "description": "",
    "overrideItemFilters": false, "overrideCurrencies": false, "overrideSecondaryCurrencies": false,
    "displayOne": false, "showItemName": false,
    "shareItemsEnabled": false, "shareCurrenciesEnabled": true, "takeAllEnabled": false,
    "splitAllEnabled": true, "activePlayers": false, "closed": false, "locked": false,
    "infiniteQuantity": false, "infiniteCurrencies": true, "purchaseOnly": false,
    "hideNewItems": false, "hideItemsWithZeroCost": false, "keepZeroQuantity": false,
    "onlyAcceptBasePrice": true, "displayQuantity": "yes", "buyPriceModifier": 1,
    "sellPriceModifier": 0.5, "itemTypePriceModifiers": [], "actorPriceModifiers": [],
    "tablesForPopulate": [], "hideTokenWhenClosed": false,
    "openTimes": {"enabled": false, "status": "open", "open": {"hour": 9, "minute": 0}, "close": {"hour": 18, "minute": 0}},
    "refreshItemsOnOpen": false, "logMerchantActivity": false,
    "cols": 10, "rows": 5, "restrictVaultAccess": false, "vaultExpansion": false
}"#;

/// Fusion profonde (objets seulement ; les tableaux sont remplacés), comme
/// foundry.utils.mergeObject.
fn deep_merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(slot) if slot.is_object() && v.is_object() => deep_merge(slot, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// Réglages effectifs : défauts du cœur ← réglage pileDefaults ← flags.
pub fn effective_config(stored: &Value, pile_defaults: Option<&Value>) -> Value {
    let mut cfg = core_defaults();
    if let Some(d) = pile_defaults.filter(|d| d.is_object()) {
        deep_merge(&mut cfg, d);
    }
    deep_merge(&mut cfg, stored);
    cfg
}

/// Les réglages présentés : communs + ceux du type de pile.
fn present_config(cfg: &Value) -> Value {
    let kind = cfg.get("type").and_then(Value::as_str).unwrap_or("pile");
    let mut keys: Vec<&str> = vec![
        "enabled",
        "type",
        "distance",
        "deleteWhenEmpty",
        "canStackItems",
        "canInspectItems",
        "description",
        "displayOne",
        "showItemName",
        "shareItemsEnabled",
        "shareCurrenciesEnabled",
        "takeAllEnabled",
        "splitAllEnabled",
        "overrideItemFilters",
        "overrideCurrencies",
    ];
    match kind {
        "container" => keys.extend([
            "closed",
            "locked",
            "closedImage",
            "openedImage",
            "emptyImage",
            "lockedImage",
        ]),
        "merchant" => keys.extend([
            "infiniteQuantity",
            "infiniteCurrencies",
            "purchaseOnly",
            "hideNewItems",
            "hideItemsWithZeroCost",
            "keepZeroQuantity",
            "onlyAcceptBasePrice",
            "displayQuantity",
            "buyPriceModifier",
            "sellPriceModifier",
            "itemTypePriceModifiers",
            "actorPriceModifiers",
            "tablesForPopulate",
            "openTimes",
            "refreshItemsOnOpen",
            "hideTokenWhenClosed",
        ]),
        "vault" => keys.extend(["cols", "rows", "restrictVaultAccess", "vaultExpansion"]),
        _ => {}
    }
    let mut out = Map::new();
    for k in keys {
        if let Some(v) = cfg.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    out.insert("hasMacro".into(), json!(non_empty(cfg.get("macro"))));
    Value::Object(out)
}

/// Objets d'un acteur synthétique (token non lié) : ceux de l'acteur de base,
/// remplacés/complétés par ceux du delta, moins les pierres tombales.
pub fn merge_delta_items(base: &[Value], delta: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = base.to_vec();
    for d in delta {
        let id = d.get("_id");
        out.retain(|b| b.get("_id") != id);
        if d.get("_tombstone").and_then(Value::as_bool) != Some(true) {
            out.push(d.clone());
        }
    }
    out
}

/// Un filtre d'objets Item Piles ({path, filters:"a,b"}) écarte-t-il cet objet ?
fn item_filtered(item: &Value, filters: &[Value]) -> bool {
    filters.iter().any(|f| {
        let Some(path) = f.get("path").and_then(Value::as_str) else {
            return false;
        };
        let Some(actual) = get_path(item, path.trim()) else {
            return false;
        };
        let wanted: Vec<Value> = match f.get("filters") {
            Some(Value::String(s)) => s
                .split(',')
                .map(|x| {
                    let x = x.trim();
                    match x.to_ascii_lowercase().as_str() {
                        "true" => json!(true),
                        "false" => json!(false),
                        _ => json!(x),
                    }
                })
                .collect(),
            Some(Value::Array(a)) => a.clone(),
            _ => vec![],
        };
        let matches = |v: &Value| wanted.contains(v);
        match actual {
            Value::Array(a) => a.iter().any(matches),
            v => matches(v),
        }
    })
}

/// Chemin parent d'un attribut (« system.price.value » → « system.price »).
fn parent_path(path: &str) -> Option<&str> {
    path.rsplit_once('.').map(|(p, _)| p)
}

/// Contenu d'une pile : objets (hors monnaies-objets et objets filtrés) et
/// monnaies. `owner` = l'acteur (système fusionné pour un token non lié).
pub fn pile_contents(
    owner: &Value,
    items: &[Value],
    ip: &Map<String, Value>,
    cfg: &Value,
) -> Value {
    let qty_path = ip
        .get("itemQuantityAttribute")
        .and_then(Value::as_str)
        .unwrap_or("");
    let price_path = ip
        .get("itemPriceAttribute")
        .and_then(Value::as_str)
        .unwrap_or("");
    let filters: Vec<Value> = match cfg.get("overrideItemFilters") {
        Some(Value::Array(a)) => a.clone(),
        _ => ip
            .get("itemFilters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    };
    let currency_list: Vec<Value> = match cfg.get("overrideCurrencies") {
        Some(Value::Array(a)) => a.clone(),
        _ => ip
            .get("currencies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    };
    let secondary: Vec<Value> = ip
        .get("secondaryCurrencies")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let quantity_of = |it: &Value| get_path(it, qty_path).cloned().unwrap_or(json!(1));

    let mut currency_item_ids: Vec<Value> = Vec::new();
    let mut currencies = Vec::new();
    for (c, is_secondary) in currency_list
        .iter()
        .map(|c| (c, false))
        .chain(secondary.iter().map(|c| (c, true)))
    {
        let kind = c.get("type").and_then(Value::as_str).unwrap_or("attribute");
        let mut entry = json!({
            "name": c.get("name"), "abbreviation": c.get("abbreviation"),
            "secondary": is_secondary, "type": kind,
        });
        if kind == "attribute" {
            let path = c
                .pointer("/data/path")
                .or_else(|| c.get("path"))
                .and_then(Value::as_str)
                .unwrap_or("");
            entry["path"] = json!(path);
            entry["quantity"] = get_path(owner, path).cloned().unwrap_or(json!(0));
        } else {
            // Monnaie-objet : appariée par nom + type, comme findSimilarItem.
            let wanted = c.pointer("/data/item");
            let found = wanted.and_then(|w| {
                items
                    .iter()
                    .find(|it| it.get("name") == w.get("name") && it.get("type") == w.get("type"))
            });
            match found {
                Some(it) => {
                    currency_item_ids.push(it.get("_id").cloned().unwrap_or(Value::Null));
                    entry["itemId"] = it.get("_id").cloned().unwrap_or(Value::Null);
                    entry["quantity"] = quantity_of(it);
                }
                None => {
                    entry["quantity"] = json!(0);
                    if wanted.is_none() {
                        entry["note"] = json!(
                            "item currency defined only by UUID — matched by the companion, not readable server-side"
                        );
                    }
                }
            }
        }
        currencies.push(entry);
    }

    let mut listed = Vec::new();
    let mut ignored = Vec::new();
    for it in items {
        if currency_item_ids.contains(it.get("_id").unwrap_or(&Value::Null)) {
            continue;
        }
        if item_filtered(it, &filters) {
            ignored
                .push(json!({"id": it.get("_id"), "name": it.get("name"), "type": it.get("type")}));
            continue;
        }
        let flags = it
            .pointer("/flags/item-piles/item")
            .cloned()
            .unwrap_or(json!({}));
        let mut e = json!({
            "id": it.get("_id"), "name": it.get("name"), "type": it.get("type"), "img": it.get("img"),
            "quantity": quantity_of(it),
            "price": get_path(it, price_path),
        });
        if let Some(p) = parent_path(price_path).and_then(|p| get_path(it, p))
            && p.is_object()
        {
            e["priceData"] = p.clone();
        }
        for k in [
            "hidden",
            "notForSale",
            "infiniteQuantity",
            "free",
            "isService",
            "customCategory",
            "keepOnMerchant",
            "buyPriceModifier",
            "sellPriceModifier",
            "prices",
        ] {
            if let Some(v) = flags
                .get(k)
                .filter(|v| non_empty(Some(v)) && **v != json!(false) && **v != json!("default"))
            {
                e[k] = v.clone();
            }
        }
        if non_empty(flags.get("macro")) {
            e["hasMacro"] = json!(true);
        }
        listed.push(e);
    }
    json!({"items": listed, "currencies": currencies, "ignoredItems": ignored})
}

/// Une cible « Scene.s.Token.t » → (scène, token).
fn parse_token_uuid(uuid: &str) -> Option<(&str, &str)> {
    let p: Vec<&str> = uuid.split('.').collect();
    (p.len() == 4 && p[0] == "Scene" && p[2] == "Token").then(|| (p[1], p[3]))
}

/// Données de pile stockées d'un token / acteur.
fn pile_flags(doc: &Value) -> Option<&Value> {
    doc.pointer("/flags/item-piles/data")
        .filter(|v| v.is_object())
}

fn is_pile(doc: &Value) -> bool {
    pile_flags(doc)
        .and_then(|d| d.get("enabled"))
        .and_then(Value::as_bool)
        == Some(true)
}

fn pile_kind(doc: &Value) -> &str {
    pile_flags(doc)
        .and_then(|d| d.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("pile")
}

// --------------------------------------------------------------- accès monde

/// Tous les réglages `item-piles.*` (clé sans préfixe) et la config des modules.
async fn load_settings(
    foundry: &FoundryHandle,
) -> Result<(Map<String, Value>, Map<String, Value>)> {
    let docs = foundry
        .get_documents(
            "settings",
            None,
            Some(&["key".into(), "value".into()]),
            0,
            None,
        )
        .await?;
    let mut ip = Map::new();
    let mut modules = Map::new();
    for d in docs {
        let Some(key) = d.get("key").and_then(Value::as_str) else {
            continue;
        };
        let val = d.get("value").map(decode_setting).unwrap_or(Value::Null);
        if key == "core.moduleConfiguration" {
            if let Value::Object(m) = val {
                modules = m;
            }
        } else if let Some(k) = key.strip_prefix("item-piles.") {
            ip.insert(k.to_string(), val);
        }
    }
    Ok((modules, ip))
}

async fn system_id(foundry: &FoundryHandle) -> String {
    crate::foundry::auth_status(&foundry.http, &foundry.hostname())
        .await
        .and_then(|s| s.get("system").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| "unknown".into())
}

async fn status(foundry: &FoundryHandle) -> Result<(Value, Map<String, Value>)> {
    let (modules, ip) = load_settings(foundry).await?;
    let sys = system_id(foundry).await;
    Ok((assess_status(&sys, &modules, &ip), ip))
}

/// Refus commun des client_ip_* quand Item Piles n'est pas prêt.
async fn ensure_ready(foundry: &FoundryHandle) -> Result<()> {
    let (st, _) = status(foundry).await?;
    if st["ready"] != json!(true) {
        bail!(
            "{}",
            st["message"].as_str().unwrap_or("Item Piles is not ready")
        );
    }
    Ok(())
}

async fn find_scene(foundry: &FoundryHandle, scene: Option<&str>) -> Result<Value> {
    let found = match scene {
        Some(s) => foundry.find_document("scenes", s, None).await?,
        None => {
            let w = json!({"active": true}).as_object().cloned().unwrap();
            foundry
                .get_documents("scenes", Some(&w), None, 0, Some(1))
                .await?
                .into_iter()
                .next()
        }
    };
    found.ok_or_else(|| match scene {
        Some(s) => anyhow!("scene « {s} » not found"),
        None => anyhow!("no active scene — pass `scene`"),
    })
}

/// Résout une cible en (document porteur des flags, acteur effectif, objets, uuid).
async fn resolve_pile(
    foundry: &FoundryHandle,
    target: &str,
) -> Result<(Value, Value, Vec<Value>, String, Value)> {
    if let Some((sid, tid)) = parse_token_uuid(target) {
        let scene = foundry
            .get_document("scenes", Some(sid), None, None)
            .await?
            .ok_or_else(|| anyhow!("scene {sid} not found"))?;
        let token = scene
            .get("tokens")
            .and_then(Value::as_array)
            .and_then(|ts| {
                ts.iter()
                    .find(|t| t.get("_id").and_then(Value::as_str) == Some(tid))
            })
            .cloned()
            .ok_or_else(|| anyhow!("token {tid} not found on scene {sid}"))?;
        let actor_id = token.get("actorId").and_then(Value::as_str).unwrap_or("");
        let base = if actor_id.is_empty() {
            None
        } else {
            foundry
                .get_document("actors", Some(actor_id), None, None)
                .await?
        };
        let base = base.unwrap_or_else(|| json!({"items": [], "system": {}}));
        let linked = token
            .get("actorLink")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let base_items = base
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let info = json!({"kind": "token", "scene": sid, "token": tid, "actorUuid": format!("Actor.{actor_id}"), "linked": linked, "x": token.get("x"), "y": token.get("y"), "tokenName": token.get("name")});
        if linked {
            // Acteur réel : ses flags font foi (le token en garde une copie).
            let holder = if pile_flags(&base).is_some() {
                base.clone()
            } else {
                token.clone()
            };
            return Ok((holder, base.clone(), base_items, target.to_string(), info));
        }
        let delta = token.get("delta").cloned().unwrap_or(json!({}));
        let delta_items = delta
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let items = merge_delta_items(&base_items, &delta_items);
        let mut owner = base.clone();
        if let Some(sys) = delta.get("system").filter(|s| s.is_object()) {
            deep_merge(&mut owner["system"], sys);
        }
        let holder = if pile_flags(&token).is_some() {
            token.clone()
        } else {
            owner.clone()
        };
        return Ok((holder, owner, items, target.to_string(), info));
    }
    let id = target.strip_prefix("Actor.").unwrap_or(target);
    let actor = foundry
        .find_document("actors", id, None)
        .await?
        .ok_or_else(|| anyhow!("pile « {target} » not found (Actor UUID, token UUID Scene.s.Token.t, or actor id/name)"))?;
    let uuid = format!(
        "Actor.{}",
        actor.get("_id").and_then(Value::as_str).unwrap_or("")
    );
    let items = actor
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let info = json!({"kind": "actor"});
    Ok((actor.clone(), actor, items, uuid, info))
}

/// Normalise `currencies` (chaîne ou objet) en chaîne Item Piles.
pub fn currency_string(v: &Value) -> Result<Option<String>> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::String(s) => Ok(Some(s.trim().to_string())),
        Value::Object(o) => {
            let mut parts = Vec::new();
            for (k, n) in o {
                let Some(n) = n.as_f64() else {
                    bail!("currencies.{k} must be a number")
                };
                if n < 0.0 {
                    bail!("currencies.{k} must be ≥ 0");
                }
                if n > 0.0 {
                    parts.push(format!("{n}{k}"));
                }
            }
            Ok((!parts.is_empty()).then(|| parts.join(" ")))
        }
        _ => bail!("currencies must be a string (\"50gp 3sp\") or an object ({{\"gp\":50}})"),
    }
}

/// Champs d'objets acceptés (uuid + quantité + quelques drapeaux Item Piles).
fn normalize_items(v: Option<&Value>) -> Result<Vec<Value>> {
    let Some(v) = v else { return Ok(vec![]) };
    let Some(list) = v.as_array() else {
        bail!("items must be an array")
    };
    list.iter()
        .enumerate()
        .map(|(i, it)| {
            let uuid = it
                .get("uuid")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| anyhow!("items[{i}].uuid is required"))?;
            let q = it
                .get("quantity")
                .map(|q| q.as_i64().filter(|n| *n >= 1))
                .unwrap_or(Some(1));
            let Some(q) = q else {
                bail!("items[{i}].quantity must be an integer ≥ 1")
            };
            let mut out = json!({"uuid": uuid, "quantity": q});
            carry(
                it,
                &["hidden", "not_for_sale", "infinite_quantity", "free"],
                &mut out,
            );
            Ok(out)
        })
        .collect()
}

fn token_ids(args: &Value) -> Result<Vec<String>> {
    let ids: Vec<String> = args
        .get("token_ids")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        bail!("token_ids is required (ids of tokens on the scene)");
    }
    Ok(ids)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "ip_status" => {
            let (st, _) = status(foundry).await?;
            Ok(text_response(&st))
        }
        "ip_list_piles" => {
            let scope = str_arg(args, "scope").unwrap_or_else(|| "all".into());
            if !["world", "scene", "all"].contains(&scope.as_str()) {
                bail!("scope must be world, scene or all");
            }
            let want = str_arg(args, "type");
            if let Some(t) = &want
                && !PILE_TYPES.contains(&t.as_str())
            {
                bail!("type must be one of {}", PILE_TYPES.join(", "));
            }
            let keep = |d: &Value| is_pile(d) && want.as_deref().is_none_or(|t| pile_kind(d) == t);
            let mut out = json!({});
            if scope != "scene" {
                let actors = foundry
                    .get_documents(
                        "actors",
                        None,
                        Some(&[
                            "type".into(),
                            "flags".into(),
                            "items".into(),
                            "folder".into(),
                        ]),
                        0,
                        None,
                    )
                    .await?;
                let list: Vec<Value> = actors
                    .iter()
                    .filter(|a| keep(a))
                    .map(|a| {
                        let f = pile_flags(a).unwrap();
                        json!({
                            "uuid": format!("Actor.{}", a["_id"].as_str().unwrap_or("")),
                            "name": a.get("name"), "pileType": pile_kind(a), "actorType": a.get("type"),
                            "closed": f.get("closed"), "locked": f.get("locked"),
                            "itemCount": a.get("items").and_then(Value::as_array).map(Vec::len),
                            "folder": a.get("folder"),
                        })
                    })
                    .collect();
                out["actors"] = json!(list);
            }
            if scope != "world" {
                let scene = find_scene(foundry, str_arg(args, "scene").as_deref()).await?;
                let sid = scene["_id"].as_str().unwrap_or("").to_string();
                let tokens = scene
                    .get("tokens")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                // Token lié sans copie des flags : on regarde l'acteur.
                let mut list = Vec::new();
                for t in &tokens {
                    let linked = t.get("actorLink").and_then(Value::as_bool).unwrap_or(false);
                    let mut holder = t.clone();
                    if linked
                        && pile_flags(t).is_none()
                        && let Some(aid) = t.get("actorId").and_then(Value::as_str)
                        && let Some(a) = foundry
                            .get_document("actors", Some(aid), None, Some(&["flags".into()]))
                            .await?
                    {
                        holder = a;
                    }
                    if !keep(&holder) {
                        continue;
                    }
                    let f = pile_flags(&holder).unwrap();
                    list.push(json!({
                        "uuid": format!("Scene.{sid}.Token.{}", t["_id"].as_str().unwrap_or("")),
                        "name": t.get("name"), "pileType": pile_kind(&holder),
                        "closed": f.get("closed"), "locked": f.get("locked"),
                        "x": t.get("x"), "y": t.get("y"), "hidden": t.get("hidden"),
                        "linked": linked,
                        "actorUuid": t.get("actorId").and_then(Value::as_str).map(|a| format!("Actor.{a}")),
                    }));
                }
                out["scene"] = json!({"id": sid, "name": scene.get("name"), "tokens": list});
            }
            Ok(text_response(&out))
        }
        "ip_get_pile" => {
            let target = str_arg(args, "uuid").ok_or_else(|| anyhow!("'uuid' is required"))?;
            let (st, ip) = status(foundry).await?;
            let (holder, owner, items, uuid, info) = resolve_pile(foundry, &target).await?;
            let stored = pile_flags(&holder).cloned().unwrap_or(json!({}));
            let cfg = effective_config(&stored, ip.get("pileDefaults"));
            let contents = pile_contents(&owner, &items, &ip, &cfg);
            let mut out = json!({
                "uuid": uuid, "name": holder.get("name").or(info.get("tokenName")), "isPile": cfg["enabled"] == json!(true),
                "target": info, "config": present_config(&cfg),
                "items": contents["items"], "currencies": contents["currencies"],
                "ignoredItems": contents["ignoredItems"],
            });
            if st["configured"] != json!(true) {
                out["warning"] = st["message"].clone();
            }
            if cfg["enabled"] != json!(true) {
                out["note"] = json!(
                    "not an enabled Item Piles pile — client_ip_turn_into_pile (token) or client_ip_create_pile"
                );
            }
            Ok(text_response(&out))
        }
        "client_ip_create_pile" => {
            let kind = str_arg(args, "type")
                .ok_or_else(|| anyhow!("'type' is required ({})", PILE_TYPES.join(", ")))?;
            if !PILE_TYPES.contains(&kind.as_str()) {
                bail!("type must be one of {}", PILE_TYPES.join(", "));
            }
            let pname = str_arg(args, "name")
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| anyhow!("'name' is required"))?;
            let has_x = args.get("x").is_some_and(Value::is_number);
            let has_y = args.get("y").is_some_and(Value::is_number);
            if has_x != has_y {
                bail!("give both x and y (scene pixels), or neither to create the actor only");
            }
            let config = match args.get("config") {
                Some(c) => validate_config(c)?,
                None => json!({}),
            };
            let mut a = json!({"type": kind, "name": pname, "config": config, "items": normalize_items(args.get("items"))?});
            if let Some(c) = currency_string(args.get("currencies").unwrap_or(&Value::Null))? {
                a["currencies"] = json!(c);
            }
            carry(
                args,
                &["x", "y", "scene_id", "img", "linked", "folder"],
                &mut a,
            );
            ensure_ready(foundry).await?;
            let r = call_companion(state, cmd_for(name), a, None, 45).await?;
            Ok(text_response(&r))
        }
        "client_ip_add_items" | "client_ip_remove_items" => {
            let target = str_arg(args, "uuid").ok_or_else(|| anyhow!("'uuid' is required"))?;
            let mut a = json!({"uuid": target});
            if name == "client_ip_add_items" {
                a["items"] = json!(normalize_items(args.get("items"))?);
            } else {
                let list = args
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for (i, it) in list.iter().enumerate() {
                    if str_arg(it, "id").is_none() && str_arg(it, "name").is_none() {
                        bail!(
                            "items[{i}] needs the item id on the pile (ip_get_pile) or its exact name"
                        );
                    }
                }
                a["items"] = json!(list);
                carry(args, &["change"], &mut a);
            }
            if let Some(c) = currency_string(args.get("currencies").unwrap_or(&Value::Null))? {
                a["currencies"] = json!(c);
            }
            if a["items"].as_array().is_none_or(Vec::is_empty) && a.get("currencies").is_none() {
                bail!("nothing to do: give items and/or currencies");
            }
            ensure_ready(foundry).await?;
            let r = call_companion(state, cmd_for(name), a, None, 45).await?;
            Ok(text_response(&r))
        }
        "client_ip_configure_pile" => {
            let target = str_arg(args, "uuid").ok_or_else(|| anyhow!("'uuid' is required"))?;
            let mut a = json!({"uuid": target});
            if let Some(c) = args.get("config") {
                a["config"] = validate_config(c)?;
            }
            if let Some(pm) = args.get("price_modifiers") {
                let Some(list) = pm.as_array() else {
                    bail!("price_modifiers must be an array")
                };
                for (i, m) in list.iter().enumerate() {
                    if str_arg(m, "actor_uuid").is_none() {
                        bail!("price_modifiers[{i}].actor_uuid is required");
                    }
                    for k in ["buy", "sell"] {
                        if m.get(k).is_some_and(|v| !v.is_number()) {
                            bail!("price_modifiers[{i}].{k} must be a number");
                        }
                    }
                }
                a["price_modifiers"] = pm.clone();
            }
            if a.get("config").is_none() && a.get("price_modifiers").is_none() {
                bail!("nothing to do: give config and/or price_modifiers");
            }
            ensure_ready(foundry).await?;
            let r = call_companion(state, cmd_for(name), a, None, 30).await?;
            Ok(text_response(&r))
        }
        "client_ip_turn_into_pile" | "client_ip_revert_pile" => {
            let mut a = json!({"token_ids": token_ids(args)?});
            carry(args, &["scene_id"], &mut a);
            if name == "client_ip_turn_into_pile" {
                let mut config = match args.get("config") {
                    Some(c) => validate_config(c)?,
                    None => json!({}),
                };
                if let Some(t) = str_arg(args, "type") {
                    if !PILE_TYPES.contains(&t.as_str()) {
                        bail!("type must be one of {}", PILE_TYPES.join(", "));
                    }
                    config["type"] = json!(t);
                }
                a["config"] = config;
            }
            ensure_ready(foundry).await?;
            let r = call_companion(state, cmd_for(name), a, None, 30).await?;
            Ok(text_response(&r))
        }
        "client_ip_restock" => {
            let target = str_arg(args, "uuid").ok_or_else(|| anyhow!("'uuid' is required"))?;
            let mut a = json!({"uuid": target});
            carry(args, &["table", "times_to_roll", "remove_existing"], &mut a);
            ensure_ready(foundry).await?;
            let r = call_companion(state, cmd_for(name), a, None, 45).await?;
            Ok(text_response(&r))
        }
        _ => bail!("Unknown tool: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dnd5e_settings() -> Map<String, Value> {
        json!({
            "currencies": [
                {"type":"attribute","name":"DND5E.CurrencyGP","abbreviation":"{#}GP","data":{"path":"system.currency.gp"},"primary":true,"exchangeRate":1},
                {"type":"attribute","name":"DND5E.CurrencySP","abbreviation":"{#}SP","data":{"path":"system.currency.sp"},"exchangeRate":0.1},
            ],
            "itemQuantityAttribute": "system.quantity",
            "itemPriceAttribute": "system.price.value",
            "actorClassType": "character",
            "itemFilters": [{"path":"type","filters":"spell,feat,class"}],
        })
        .as_object()
        .cloned()
        .unwrap()
    }

    #[test]
    fn statut_pret_inactif_ou_non_configure() {
        let modules = json!({"item-piles": true, "itempilesdnd5e": true})
            .as_object()
            .cloned()
            .unwrap();
        let st = assess_status("dnd5e", &modules, &dnd5e_settings());
        assert_eq!(st["ready"], json!(true));
        assert_eq!(st["integrationModules"], json!(["itempilesdnd5e"]));
        assert_eq!(st["currencies"][0]["path"], json!("system.currency.gp"));

        let off = json!({"item-piles": false}).as_object().cloned().unwrap();
        let st = assess_status("dnd5e", &off, &dnd5e_settings());
        assert_eq!(st["ready"], json!(false));
        assert!(st["message"].as_str().unwrap().contains("not active"));

        let st = assess_status(
            "starwarsffg",
            &json!({"item-piles": true}).as_object().cloned().unwrap(),
            &Map::new(),
        );
        assert_eq!(st["ready"], json!(false));
        let msg = st["message"].as_str().unwrap();
        assert!(msg.contains("maersk-aerospace"), "{msg}");
        let st = assess_status(
            "cyberpunk",
            &json!({"item-piles": true}).as_object().cloned().unwrap(),
            &Map::new(),
        );
        assert!(st["message"].as_str().unwrap().contains("by hand"));
    }

    #[test]
    fn config_liste_blanche_et_macro_refusee() {
        assert!(
            validate_config(&json!({"closed": true, "buyPriceModifier": 1.5, "type": "merchant"}))
                .is_ok()
        );
        let e = validate_config(&json!({"macro": "Macro.x"}))
            .unwrap_err()
            .to_string();
        assert!(e.contains("refused"), "{e}");
        assert!(validate_config(&json!({"type": "shop"})).is_err());
        assert!(validate_config(&json!({"sellPriceModifier": -1})).is_err());
        assert!(validate_config(&json!({"whatever": 1})).is_err());
        assert!(validate_config(&json!({"enabled": false})).is_err());
        assert!(validate_config(&json!([1])).is_err());
    }

    #[test]
    fn monnaies_chaine_ou_objet() {
        assert_eq!(
            currency_string(&json!("50gp 3sp")).unwrap().unwrap(),
            "50gp 3sp"
        );
        assert_eq!(
            currency_string(&json!({"gp": 50, "sp": 0}))
                .unwrap()
                .unwrap(),
            "50gp"
        );
        assert!(currency_string(&json!({"gp": -1})).is_err());
        assert!(currency_string(&json!("")).unwrap().is_none());
        assert!(currency_string(&json!(5)).is_err());
    }

    #[test]
    fn config_effective_fusionnee() {
        let stored = json!({"enabled": true, "type": "merchant", "buyPriceModifier": 1.5, "openTimes": {"enabled": true}});
        let cfg = effective_config(&stored, Some(&json!({"sellPriceModifier": 0.25})));
        assert_eq!(cfg["buyPriceModifier"], json!(1.5));
        assert_eq!(cfg["sellPriceModifier"], json!(0.25));
        assert_eq!(cfg["openTimes"]["status"], json!("open"));
        assert_eq!(cfg["openTimes"]["enabled"], json!(true));
        let p = present_config(&cfg);
        assert!(p.get("infiniteQuantity").is_some());
        assert!(p.get("closed").is_none());
        assert_eq!(p["hasMacro"], json!(false));
    }

    #[test]
    fn delta_d_un_token_non_lie() {
        let base = vec![json!({"_id":"a","name":"A"}), json!({"_id":"b","name":"B"})];
        let delta = vec![
            json!({"_id":"a","name":"A2"}),
            json!({"_id":"b","_tombstone":true}),
            json!({"_id":"c","name":"C"}),
        ];
        let m = merge_delta_items(&base, &delta);
        let names: Vec<&str> = m.iter().map(|i| i["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["A2", "C"]);
    }

    #[test]
    fn contenu_filtres_et_monnaies() {
        let owner = json!({"system": {"currency": {"gp": 50, "sp": 3}}});
        let items = vec![
            json!({"_id":"i1","name":"Torch","type":"consumable","system":{"quantity":3,"price":{"value":1,"denomination":"cp"}},
                   "flags":{"item-piles":{"item":{"hidden":true,"infiniteQuantity":"default"}}}}),
            json!({"_id":"i2","name":"Fireball","type":"spell","system":{}}),
        ];
        let ip = dnd5e_settings();
        let cfg = effective_config(&json!({"enabled": true}), None);
        let c = pile_contents(&owner, &items, &ip, &cfg);
        assert_eq!(c["items"].as_array().unwrap().len(), 1);
        assert_eq!(c["items"][0]["quantity"], json!(3));
        assert_eq!(c["items"][0]["price"], json!(1));
        assert_eq!(c["items"][0]["priceData"]["denomination"], json!("cp"));
        assert_eq!(c["items"][0]["hidden"], json!(true));
        assert!(c["items"][0].get("infiniteQuantity").is_none());
        assert_eq!(c["ignoredItems"][0]["name"], json!("Fireball"));
        assert_eq!(c["currencies"][0]["quantity"], json!(50));
        assert_eq!(c["currencies"][1]["quantity"], json!(3));
    }

    #[test]
    fn monnaie_objet_appariee_par_nom_et_type() {
        let mut ip = dnd5e_settings();
        ip.insert("currencies".into(), json!([{"type":"item","name":"Credits","abbreviation":"{#}cr","data":{"item":{"name":"Credit chip","type":"loot"}},"primary":true,"exchangeRate":1}]));
        let items = vec![
            json!({"_id":"c","name":"Credit chip","type":"loot","system":{"quantity":120}}),
            json!({"_id":"d","name":"Rope","type":"loot","system":{"quantity":1}}),
        ];
        let c = pile_contents(&json!({}), &items, &ip, &effective_config(&json!({}), None));
        assert_eq!(c["currencies"][0]["quantity"], json!(120));
        assert_eq!(c["items"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn objets_normalises() {
        let v = normalize_items(Some(
            &json!([{"uuid":"Item.a"},{"uuid":"Item.b","quantity":3,"hidden":true}]),
        ))
        .unwrap();
        assert_eq!(v[0]["quantity"], json!(1));
        assert_eq!(v[1]["hidden"], json!(true));
        assert!(normalize_items(Some(&json!([{"quantity":2}]))).is_err());
        assert!(normalize_items(Some(&json!([{"uuid":"Item.a","quantity":0}]))).is_err());
        assert_eq!(parse_token_uuid("Scene.s.Token.t"), Some(("s", "t")));
        assert_eq!(parse_token_uuid("Actor.a"), None);
    }
}
