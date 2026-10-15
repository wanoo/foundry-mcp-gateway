//! Campaign Codex (cc_*) — fiches = JournalEntry + flags["campaign-codex"],
//! et délégations au compagnon (API navigateur du module).
//!
//! Format visé : Campaign Codex 6.9 (« contentStorageVersion 1 ») — la prose
//! (Information, GM Notes, onglets personnalisés) vit dans des pages gérées
//! (`flags.campaign-codex.contentKey`), indexées par `flags.campaign-codex.contentPages`
//! sur le journal ; `data.description` / `data.notes` ne sont plus qu'un repli.
//!
//! Les hooks de Campaign Codex ne tournent que chez l'utilisateur auteur de la
//! modification : la passerelle (client socket sans navigateur) écrit donc
//! elle-même LES DEUX CÔTÉS de chaque lien, comme le ferait `CampaignManager`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::foundry::documents::get_path;
use crate::mcp::McpState;
use crate::tools::companion::{Delivery, call_companion, cmd_for};
use crate::tools::{ToolDef, post_chat, str_arg, text_response};

const MODULE: &str = "campaign-codex";
const WEBHUB: &str = "campaign-codex-web-hub-sync";
const CONTENT_PAGE_SHEET_CLASS: &str = "campaign-codex.CampaignCodexContentPageSheet";
const CUSTOM_PREFIX: &str = "custom-info-";

pub const CC_TYPES: [&str; 7] = ["npc", "group", "location", "region", "shop", "quest", "tag"];

/// Types acceptés par `convertJournalToCCSheet` (helper.js).
const CONVERT_TYPES: [&str; 5] = ["location", "npc", "region", "shop", "group"];

/// Classe de feuille Campaign Codex par type (main.js, CODEX_SHEET_CLASS_BY_TYPE).
fn sheet_class(cc_type: &str) -> Option<&'static str> {
    Some(match cc_type {
        "region" => "campaign-codex.RegionSheet",
        "location" => "campaign-codex.LocationSheet",
        "shop" => "campaign-codex.ShopSheet",
        "npc" => "campaign-codex.NPCSheet",
        "tag" => "campaign-codex.TagSheet",
        "group" => "campaign-codex.GroupSheet",
        "quest" => "campaign-codex.QuestSheet",
        _ => return None,
    })
}

/// Type CC d'un journal : flag explicite, sinon déduit de la classe de feuille
/// (même logique que `getCodexType`).
fn cc_type_of(doc: &Value) -> Option<String> {
    if let Some(t) = get_path(doc, "flags.campaign-codex.type").and_then(Value::as_str)
        && !t.is_empty()
    {
        return Some(t.to_string());
    }
    let class = get_path(doc, "flags.core.sheetClass").and_then(Value::as_str)?;
    let short = class
        .strip_prefix("campaign-codex.")?
        .strip_suffix("Sheet")?;
    let t = short.to_lowercase();
    CC_TYPES.contains(&t.as_str()).then_some(t)
}

/// Identifiant Foundry (16 caractères alphanumériques, comme `foundry.utils.randomID`).
fn random_id() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    (0..16)
        .map(|_| CHARS[(rand::random::<u32>() as usize) % CHARS.len()] as char)
        .collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn journal_uuid(id: &str) -> String {
    format!("JournalEntry.{id}")
}

/// `JournalEntry.<id>[.JournalEntryPage.<pid>]` → `<id>` (sinon None).
fn journal_id_of_uuid(uuid: &str) -> Option<&str> {
    let rest = uuid.strip_prefix("JournalEntry.")?;
    let id = rest.split('.').next()?;
    (!id.is_empty()).then_some(id)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Pose `v` au chemin pointé `path` dans `obj` (crée les objets intermédiaires).
fn set_nested(obj: &mut Value, path: &str, v: Value) {
    let mut cur = obj;
    let segs: Vec<&str> = path.split('.').collect();
    for (i, seg) in segs.iter().enumerate() {
        if !cur.is_object() {
            *cur = json!({});
        }
        let map = cur.as_object_mut().expect("objet");
        if i == segs.len() - 1 {
            map.insert((*seg).to_string(), v);
            return;
        }
        cur = map.entry((*seg).to_string()).or_insert_with(|| json!({}));
    }
}

// ------------------------------------------------------------ structure d'une fiche

/// Quête par défaut (campaign-manager.js, createDefaultQuestData).
fn default_quest(title: &str, id: &str, now: i64) -> Value {
    json!({
        "id": id, "title": title, "description": "",
        "inactive": true, "completed": false, "failed": false, "visible": false,
        "pinned": false, "hideRewards": false, "notifyPlayers": false,
        "messageOnCompleted": false, "urgency": "medium", "boardColumn": "active",
        "questGiverUuid": "", "relatedUuids": [], "dependencies": [], "unlocks": [],
        "checkIns": [], "linkedMacros": [],
        "rewardXP": 0, "rewardCurrency": 0, "rewardReputation": 0, "rewardClaimed": false,
        "updatedAt": now, "objectives": [], "inventory": [],
    })
}

/// `flags.campaign-codex.data` initial, exactement comme les create*Journal de CC.
fn data_defaults(cc_type: &str, quest: Option<Value>) -> Value {
    match cc_type {
        "npc" | "tag" => json!({
            "linkedActor": null, "description": "", "linkedLocations": [],
            "linkedShops": [], "associates": [], "notes": "", "tagMode": cc_type == "tag",
        }),
        "location" => json!({
            "description": "", "linkedNPCs": [], "linkedScene": null,
            "linkedShops": [], "notes": "",
        }),
        "shop" => json!({
            "description": "", "linkedNPCs": [], "linkedLocation": null, "inventory": [],
            "linkedScene": null, "markup": 1.0, "notes": "",
        }),
        "region" => json!({
            "description": "", "linkedLocations": [], "linkedScene": null, "notes": "",
        }),
        "group" => json!({"description": "", "members": [], "linkedNPCs": [], "notes": ""}),
        "quest" => json!({
            "description": "", "notes": "",
            "quests": [quest.unwrap_or_else(|| default_quest("New Quest", &random_id(), now_ms()))],
        }),
        _ => json!({}),
    }
}

/// Page de contenu gérée (journal-content-helper.js, createPageData).
fn content_page(
    key: &str,
    name: &str,
    sort: i64,
    restricted: bool,
    content: &str,
    id: &str,
) -> Value {
    json!({
        "_id": id, "name": name, "type": "text", "sort": sort,
        "title": {"show": true, "level": 1},
        "text": {"format": 1, "content": content},
        "ownership": {"default": if restricted { 0 } else { -1 }},
        "flags": {
            "core": {"sheetClass": CONTENT_PAGE_SHEET_CLASS},
            "campaign-codex": {"contentKey": key},
        },
    })
}

struct NewSheet<'a> {
    name: &'a str,
    cc_type: &'a str,
    info: &'a str,
    notes: &'a str,
    image: Option<&'a str>,
    tags: Option<Value>,
    linked_actor: Option<String>,
    player_visible: bool,
    folder: Option<String>,
}

/// Document JournalEntry complet d'une nouvelle fiche (format v6 :
/// sheetClass + contentStorageVersion 1 + pages Information / GM Notes).
fn build_sheet_doc(s: &NewSheet, info_id: &str, notes_id: &str, now: i64) -> Value {
    let quest = (s.cc_type == "quest").then(|| default_quest(s.name, &random_id(), now));
    let mut data = data_defaults(s.cc_type, quest);
    if let Some(tags) = &s.tags {
        data["tags"] = tags.clone();
    }
    if let Some(a) = &s.linked_actor
        && data.get("linkedActor").is_some()
    {
        data["linkedActor"] = json!(a);
    }
    let mut cc = json!({
        "type": s.cc_type, "data": data,
        "contentStorageVersion": 1,
        "contentPages": {"info": info_id, "notes": notes_id},
    });
    if let Some(img) = s.image {
        cc["image"] = json!(img);
    }
    let mut doc = json!({
        "name": s.name,
        "flags": {"campaign-codex": cc, "core": {"sheetClass": sheet_class(s.cc_type)}},
        "pages": [
            content_page("info", "Information", 100000, false, s.info, info_id),
            content_page("notes", "GM Notes", 200000, true, s.notes, notes_id),
        ],
        "ownership": {"default": if s.player_visible { 2 } else { 0 }},
    });
    if let Some(f) = &s.folder {
        doc["folder"] = json!(f);
    }
    doc
}

// ------------------------------------------------------------ lecture du contenu

fn storage_version(doc: &Value) -> i64 {
    get_path(doc, "flags.campaign-codex.contentStorageVersion")
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

fn page_key(page: &Value) -> Option<&str> {
    get_path(page, "flags.campaign-codex.contentKey").and_then(Value::as_str)
}

/// Page gérée d'une clé : d'abord via l'index contentPages, sinon par contentKey.
fn find_content_page<'a>(doc: &'a Value, key: &str) -> Option<&'a Value> {
    let pages = doc.get("pages").and_then(Value::as_array)?;
    if let Some(id) = get_path(doc, "flags.campaign-codex.contentPages")
        .and_then(|i| i.get(key))
        .and_then(Value::as_str)
        && let Some(p) = pages
            .iter()
            .find(|p| p["_id"].as_str() == Some(id) && page_key(p) == Some(key))
    {
        return Some(p);
    }
    pages.iter().find(|p| page_key(p) == Some(key))
}

fn legacy_path(key: &str) -> Option<String> {
    match key {
        "info" => Some("description".into()),
        "notes" => Some("notes".into()),
        k if k.starts_with(CUSTOM_PREFIX) => Some(format!("customTabContent.{k}.description")),
        _ => None,
    }
}

fn normalize_content(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.first().and_then(Value::as_str).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// Contenu d'une clé (info / notes / custom-info-*) — repli legacy si storage 0.
fn read_content(doc: &Value, key: &str) -> String {
    if let Some(p) = find_content_page(doc, key) {
        return normalize_content(get_path(p, "text.content"));
    }
    if storage_version(doc) >= 1 {
        return String::new();
    }
    legacy_path(key)
        .and_then(|lp| get_path(doc, &format!("flags.campaign-codex.data.{lp}")))
        .map(|v| normalize_content(Some(v)))
        .unwrap_or_default()
}

fn normalize_custom_key(raw: &str, index: usize) -> String {
    let raw = raw.trim().to_lowercase();
    let base = if raw.starts_with(CUSTOM_PREFIX) {
        raw
    } else if raw.is_empty() {
        format!("{CUSTOM_PREFIX}{}", index + 1)
    } else {
        format!("{CUSTOM_PREFIX}{raw}")
    };
    base.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

/// Onglets personnalisés (flag custom-info-tabs) : (clé normalisée, libellé).
fn custom_tabs(doc: &Value) -> Vec<(String, String)> {
    let Some(raw) =
        get_path(doc, "flags.campaign-codex.custom-info-tabs").and_then(Value::as_array)
    else {
        return vec![];
    };
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for (i, t) in raw.iter().enumerate() {
        let key = normalize_custom_key(t.get("key").and_then(Value::as_str).unwrap_or(""), i);
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        let label = t
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .unwrap_or_else(|| format!("Custom {}", out.len() + 1));
        out.push((key, label));
    }
    out
}

fn tab_overrides(doc: &Value) -> Vec<Value> {
    get_path(doc, "flags.campaign-codex.tab-overrides")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Page restreinte (cachée aux joueurs) ? notes toujours ; sinon override hidden.
fn key_restricted(overrides: &[Value], key: &str) -> bool {
    key == "notes"
        || overrides
            .iter()
            .find(|o| o.get("key").and_then(Value::as_str) == Some(key))
            .and_then(|o| o.get("hidden"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// Mises à jour d'ownership des pages gérées pour qu'elles collent aux règles
/// de CC (notes : NONE ; info/custom : NONE si caché, sinon INHERIT).
fn page_ownership_updates(doc: &Value, overrides: &[Value]) -> Vec<Value> {
    let Some(pages) = doc.get("pages").and_then(Value::as_array) else {
        return vec![];
    };
    pages
        .iter()
        .filter_map(|p| {
            let key = page_key(p)?;
            let want = if key_restricted(overrides, key) {
                0
            } else {
                -1
            };
            let ok = p.get("ownership") == Some(&json!({"default": want}));
            (!ok).then(|| json!({"_id": p["_id"], "==ownership": {"default": want}}))
        })
        .collect()
}

/// Réécrit l'index contentPages dans l'ordre voulu. CC compare l'index par
/// JSON.stringify (l'ordre des clés compte) et le « réparerait » sinon ; une
/// fusion ajoute les nouvelles clés en fin d'objet et serde_json trie les clés
/// à l'envoi (et le serveur applique les « -= » après les ajouts d'une même
/// mise à jour) : on vide donc l'index, puis on ajoute les clés une par une.
fn content_index_patches(current: &[String], ordered: &[(String, String)]) -> Vec<Value> {
    let deletions: Map<String, Value> = current
        .iter()
        .map(|k| (format!("-={k}"), Value::Null))
        .collect();
    let mut patches = vec![];
    if !deletions.is_empty() {
        patches.push(Value::Object(deletions));
    }
    patches.extend(ordered.iter().map(|(k, id)| json!({ k.as_str(): id })));
    patches
}

// ------------------------------------------------------------ liens

/// Nature d'une extrémité de lien.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Cc(String),
    Journal,
    Scene,
    Actor,
}

impl Kind {
    fn cc(&self, types: &[&str]) -> bool {
        matches!(self, Kind::Cc(t) if types.contains(&t.as_str()))
    }
    fn label(&self) -> String {
        match self {
            Kind::Cc(t) => format!("{t} sheet"),
            Kind::Journal => "plain journal".into(),
            Kind::Scene => "scene".into(),
            Kind::Actor => "actor".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rel {
    Actor,
    Scene,
    Journal,
    Associates,
    NpcLocation,
    NpcShop,
    TagGroup,
    ShopLocation,
    RegionLocation,
    RegionRegion,
    Member,
}

/// Ordre de déduction quand `relation` est absente.
const RELS: [Rel; 11] = [
    Rel::Actor,
    Rel::Scene,
    Rel::Journal,
    Rel::Associates,
    Rel::NpcLocation,
    Rel::NpcShop,
    Rel::TagGroup,
    Rel::ShopLocation,
    Rel::RegionLocation,
    Rel::RegionRegion,
    Rel::Member,
];

const NPCISH: [&str; 2] = ["npc", "tag"];
const PLACE: [&str; 2] = ["location", "region"];

/// Côté d'un lien stocké dans flags.campaign-codex.data : (champ, scalaire ?).
type Side = Option<(&'static str, bool)>;

impl Rel {
    fn name(self) -> &'static str {
        match self {
            Rel::Actor => "actor",
            Rel::Scene => "scene",
            Rel::Journal => "journal",
            Rel::Associates => "associates",
            Rel::NpcLocation => "npc_location",
            Rel::NpcShop => "npc_shop",
            Rel::TagGroup => "tag_group",
            Rel::ShopLocation => "shop_location",
            Rel::RegionLocation => "region_location",
            Rel::RegionRegion => "region_region",
            Rel::Member => "member",
        }
    }

    /// Ok(Some(rel)) : relation explicite ; Ok(None) : à déduire des types
    /// (valeurs historiques linkedLocations/linkedShops/linkedNPCs, ou absent).
    fn parse(s: Option<&str>) -> Result<Option<Rel>> {
        let Some(s) = s else { return Ok(None) };
        if let Some(r) = RELS.iter().find(|r| r.name() == s) {
            return Ok(Some(*r));
        }
        Ok(match s {
            "linkedActor" => Some(Rel::Actor),
            "linkedLocation" => Some(Rel::ShopLocation),
            "linkedLocations" | "linkedShops" | "linkedNPCs" => None,
            other => bail!("Unknown relation '{other}'"),
        })
    }

    /// Rôles : `a` porte le champ principal, `b` l'autre extrémité.
    fn accepts(self, a: &Kind, b: &Kind) -> bool {
        match self {
            Rel::Actor => a.cc(&NPCISH) && *b == Kind::Actor,
            Rel::Scene => matches!(a, Kind::Cc(_)) && *b == Kind::Scene,
            Rel::Journal => matches!(a, Kind::Cc(_)) && *b == Kind::Journal,
            Rel::Associates => a.cc(&NPCISH) && b.cc(&NPCISH),
            Rel::NpcLocation => a.cc(&NPCISH) && b.cc(&PLACE),
            Rel::NpcShop => a.cc(&NPCISH) && b.cc(&["shop"]),
            Rel::TagGroup => a.cc(&["tag"]) && b.cc(&["group"]),
            Rel::ShopLocation => a.cc(&PLACE) && b.cc(&["shop"]),
            Rel::RegionLocation => a.cc(&["region"]) && b.cc(&["location"]),
            Rel::RegionRegion => a.cc(&["region"]) && b.cc(&["region"]),
            Rel::Member => a.cc(&["group"]) && matches!(b, Kind::Cc(_)),
        }
    }

    /// (côté a, côté b) dans les flags CC ; None = pas de champ CC de ce côté.
    fn sides(self) -> (Side, Side) {
        match self {
            Rel::Actor => (Some(("linkedActor", true)), None),
            Rel::Scene => (Some(("linkedScene", true)), None),
            Rel::Journal => (Some(("linkedStandardJournals", false)), None),
            Rel::Associates => (Some(("associates", false)), Some(("associates", false))),
            Rel::NpcLocation => (
                Some(("linkedLocations", false)),
                Some(("linkedNPCs", false)),
            ),
            Rel::NpcShop => (Some(("linkedShops", false)), Some(("linkedNPCs", false))),
            Rel::TagGroup => (Some(("linkedGroups", false)), Some(("linkedNPCs", false))),
            Rel::ShopLocation => (Some(("linkedShops", false)), Some(("linkedLocation", true))),
            Rel::RegionLocation => (
                Some(("linkedLocations", false)),
                Some(("parentRegion", true)),
            ),
            Rel::RegionRegion => (
                Some(("linkedRegions", false)),
                Some(("parentRegions", false)),
            ),
            Rel::Member => (Some(("members", false)), None),
        }
    }
}

/// Choisit la relation et l'orientation : renvoie (rel, inversé ?) où
/// inversé = `from` joue le rôle b. region_region n'est jamais inversée
/// (`from` = parent).
fn resolve_rel(explicit: Option<Rel>, from: &Kind, to: &Kind) -> Result<(Rel, bool)> {
    let try_rel = |r: Rel| -> Option<(Rel, bool)> {
        if r.accepts(from, to) {
            Some((r, false))
        } else if r != Rel::RegionRegion && r.accepts(to, from) {
            Some((r, true))
        } else {
            None
        }
    };
    if let Some(r) = explicit {
        return try_rel(r).ok_or_else(|| {
            anyhow!(
                "Relation '{}' cannot link a {} to a {}",
                r.name(),
                from.label(),
                to.label()
            )
        });
    }
    RELS.iter().find_map(|r| try_rel(*r)).ok_or_else(|| {
        anyhow!(
            "Campaign Codex has no relation between a {} and a {} (quests link through cc_quest_update giver/related; a group can hold any sheet as member)",
            from.label(),
            to.label()
        )
    })
}

/// Fiches CC chargées + champs modifiés, pour écrire les deux côtés en un lot.
#[derive(Default)]
struct Store {
    docs: HashMap<String, Value>,
    dirty: BTreeMap<String, BTreeSet<&'static str>>,
}

impl Store {
    fn insert(&mut self, doc: Value) {
        if let Some(id) = doc["_id"].as_str() {
            self.docs.entry(id.to_string()).or_insert(doc);
        }
    }
    fn field(&self, id: &str, field: &str) -> Option<&Value> {
        self.docs
            .get(id)
            .and_then(|d| get_path(d, "flags.campaign-codex.data"))
            .and_then(|d| d.get(field))
    }
    fn list(&self, id: &str, field: &str) -> Vec<Value> {
        self.field(id, field)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }
    fn put(&mut self, id: &str, field: &'static str, v: Value) {
        if let Some(doc) = self.docs.get_mut(id) {
            set_nested(doc, &format!("flags.campaign-codex.data.{field}"), v);
            self.dirty.entry(id.to_string()).or_default().insert(field);
        }
    }
    fn add(&mut self, id: &str, field: &'static str, uuid: &str) -> bool {
        let mut l = self.list(id, field);
        if l.iter().any(|v| v.as_str() == Some(uuid)) {
            return false;
        }
        l.push(json!(uuid));
        self.put(id, field, Value::Array(l));
        true
    }
    fn remove(&mut self, id: &str, field: &'static str, uuid: &str) -> bool {
        let l = self.list(id, field);
        let n = l.len();
        let kept: Vec<Value> = l.into_iter().filter(|v| v.as_str() != Some(uuid)).collect();
        if kept.len() == n {
            return false;
        }
        self.put(id, field, Value::Array(kept));
        true
    }
    fn set(&mut self, id: &str, field: &'static str, v: Value) -> bool {
        if self.field(id, field) == Some(&v) {
            return false;
        }
        self.put(id, field, v);
        true
    }
    /// Remet un scalaire à null s'il pointe vers `uuid`.
    fn clear_if(&mut self, id: &str, field: &'static str, uuid: &str) -> bool {
        if self.field(id, field).and_then(Value::as_str) == Some(uuid) {
            self.put(id, field, Value::Null);
            return true;
        }
        false
    }
    fn link(&mut self, id: &str, side: (&'static str, bool), uuid: &str, unlink: bool) -> bool {
        match (side.1, unlink) {
            (false, false) => self.add(id, side.0, uuid),
            (false, true) => self.remove(id, side.0, uuid),
            (true, false) => self.set(id, side.0, json!(uuid)),
            (true, true) => self.clear_if(id, side.0, uuid),
        }
    }
    /// Une mise à jour (objets imbriqués) par journal modifié.
    fn updates(&self) -> Vec<Value> {
        self.dirty
            .iter()
            .map(|(id, fields)| {
                let mut data = Map::new();
                for f in fields {
                    data.insert(
                        (*f).to_string(),
                        self.field(id, f).cloned().unwrap_or(Value::Null),
                    );
                }
                json!({"_id": id, "flags": {"campaign-codex": {"data": data}}})
            })
            .collect()
    }
    fn name(&self, id: &str) -> String {
        self.docs
            .get(id)
            .and_then(|d| d["name"].as_str())
            .unwrap_or(id)
            .to_string()
    }
}

/// Ancêtres d'une région (via parentRegions), pour refuser les cycles.
fn region_ancestors(regions: &HashMap<String, Value>, start: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![start.to_string()];
    while let Some(id) = stack.pop() {
        let parents = regions
            .get(&id)
            .and_then(|d| get_path(d, "flags.campaign-codex.data.parentRegions"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for p in parents
            .iter()
            .filter_map(Value::as_str)
            .filter_map(journal_id_of_uuid)
        {
            if seen.insert(p.to_string()) {
                stack.push(p.to_string());
            }
        }
    }
    seen
}

/// Groupes contenus (récursivement) dans un groupe, via members.
fn nested_groups(sheets: &HashMap<String, Value>, start: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![start.to_string()];
    while let Some(id) = stack.pop() {
        let members = sheets
            .get(&id)
            .and_then(|d| get_path(d, "flags.campaign-codex.data.members"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for m in members
            .iter()
            .filter_map(Value::as_str)
            .filter_map(journal_id_of_uuid)
        {
            if sheets.get(m).and_then(cc_type_of).as_deref() == Some("group")
                && seen.insert(m.to_string())
            {
                stack.push(m.to_string());
            }
        }
    }
    seen
}

// ------------------------------------------------------------ quêtes

fn apply_quest_status(q: &mut Value, status: &str) -> Result<()> {
    let (inactive, completed, failed) = match status {
        "active" => (false, false, false),
        "inactive" => (true, false, false),
        "completed" => (false, true, false),
        "failed" => (false, false, true),
        other => bail!("Unknown quest status '{other}' (active | inactive | completed | failed)"),
    };
    q["inactive"] = json!(inactive);
    if status != "inactive" {
        q["completed"] = json!(completed);
        q["failed"] = json!(failed);
    }
    let column = if q["completed"].as_bool() == Some(true) {
        "completed"
    } else if q["failed"].as_bool() == Some(true) {
        "failed"
    } else {
        "active"
    };
    q["boardColumn"] = json!(column);
    Ok(())
}

fn quest_status_label(q: &Value) -> &'static str {
    if q["completed"].as_bool() == Some(true) {
        "completed"
    } else if q["failed"].as_bool() == Some(true) {
        "failed"
    } else if q["inactive"].as_bool() == Some(true) {
        "inactive"
    } else {
        "active"
    }
}

/// Journaux (ids) cités par le donneur et les liés d'une quête → linkedQuests inverses.
fn quest_backlink_ids(q: &Value, self_id: &str) -> BTreeSet<String> {
    let mut refs: Vec<&str> = vec![];
    if let Some(g) = q["questGiverUuid"].as_str() {
        refs.push(g);
    }
    if let Some(r) = q["relatedUuids"].as_array() {
        refs.extend(r.iter().filter_map(Value::as_str));
    }
    refs.into_iter()
        .filter_map(journal_id_of_uuid)
        .filter(|id| *id != self_id)
        .map(String::from)
        .collect()
}

fn find_objective<'a>(objs: &'a mut [Value], id: &str) -> Option<&'a mut Value> {
    for o in objs.iter_mut() {
        if o["id"].as_str() == Some(id) {
            return Some(o);
        }
        if let Some(children) = o.get_mut("objectives").and_then(Value::as_array_mut)
            && let Some(f) = find_objective(children, id)
        {
            return Some(f);
        }
    }
    None
}

fn remove_objective(objs: &mut Vec<Value>, id: &str) -> bool {
    let n = objs.len();
    objs.retain(|o| o["id"].as_str() != Some(id));
    if objs.len() != n {
        return true;
    }
    objs.iter_mut().any(|o| {
        o.get_mut("objectives")
            .and_then(Value::as_array_mut)
            .is_some_and(|c| remove_objective(c, id))
    })
}

fn patch_objective(o: &mut Value, args: &Value) {
    if let Some(t) = args.get("text").and_then(Value::as_str) {
        o["text"] = json!(t);
    }
    for k in ["visible"] {
        if let Some(b) = args.get(k).and_then(Value::as_bool) {
            o[k] = json!(b);
        }
    }
    // completed / failed s'excluent, comme dans la feuille de quête
    if let Some(b) = args.get("completed").and_then(Value::as_bool) {
        o["completed"] = json!(b);
        if b {
            o["failed"] = json!(false);
        }
    }
    if let Some(b) = args.get("failed").and_then(Value::as_bool) {
        o["failed"] = json!(b);
        if b {
            o["completed"] = json!(false);
        }
    }
}

/// Applique add | set | remove à l'arbre d'objectifs ; renvoie l'id touché.
fn objective_op(objs: &mut Vec<Value>, args: &Value, new_id: &str) -> Result<String> {
    let op = str_arg(args, "op").ok_or_else(|| anyhow!("'op' is required (add | set | remove)"))?;
    match op.as_str() {
        "add" => {
            let mut o = json!({
                "id": new_id,
                "text": args.get("text").and_then(Value::as_str).unwrap_or("New Objective"),
                "completed": false, "failed": false, "visible": false, "objectives": [],
            });
            patch_objective(&mut o, args);
            match str_arg(args, "parent_id") {
                Some(pid) => {
                    let parent = find_objective(objs, &pid)
                        .ok_or_else(|| anyhow!("Parent objective not found: {pid}"))?;
                    if !parent["objectives"].is_array() {
                        parent["objectives"] = json!([]);
                    }
                    parent["objectives"]
                        .as_array_mut()
                        .expect("tableau")
                        .push(o);
                }
                None => objs.push(o),
            }
            Ok(new_id.to_string())
        }
        "set" => {
            let id = str_arg(args, "id").ok_or_else(|| anyhow!("'id' is required for op=set"))?;
            let o =
                find_objective(objs, &id).ok_or_else(|| anyhow!("Objective not found: {id}"))?;
            patch_objective(o, args);
            Ok(id)
        }
        "remove" => {
            let id =
                str_arg(args, "id").ok_or_else(|| anyhow!("'id' is required for op=remove"))?;
            if !remove_objective(objs, &id) {
                bail!("Objective not found: {id}");
            }
            Ok(id)
        }
        other => bail!("Unknown op '{other}' (add | set | remove)"),
    }
}

// ------------------------------------------------------------ boutique

/// Ligne d'inventaire par itemUuid exact, id d'objet, ou nom (insensible à la casse).
fn find_inventory_row(inv: &[Value], item: &str) -> Option<usize> {
    let suffix = format!(".{item}");
    inv.iter()
        .position(|r| r["itemUuid"].as_str() == Some(item))
        .or_else(|| {
            inv.iter().position(|r| {
                r["itemId"].as_str() == Some(item)
                    || r["itemUuid"].as_str().is_some_and(|u| u.ends_with(&suffix))
            })
        })
        .or_else(|| {
            let low = item.to_lowercase();
            inv.iter()
                .position(|r| r["name"].as_str().is_some_and(|n| n.to_lowercase() == low))
        })
}

// ------------------------------------------------------------ filtres de liste

const LIST_FIELDS: [&str; 10] = [
    "associates",
    "linkedLocations",
    "linkedShops",
    "linkedNPCs",
    "linkedRegions",
    "parentRegions",
    "linkedGroups",
    "members",
    "linkedStandardJournals",
    "linkedQuests",
];
const SCALAR_FIELDS: [&str; 4] = [
    "linkedActor",
    "linkedLocation",
    "parentRegion",
    "linkedScene",
];

fn link_counts(data: &Value) -> (usize, Map<String, Value>) {
    let mut total = 0;
    let mut counts = Map::new();
    for f in LIST_FIELDS {
        let n = data
            .get(f)
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        if n > 0 {
            counts.insert(f.into(), json!(n));
            total += n;
        }
    }
    for f in SCALAR_FIELDS {
        if data
            .get(f)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            counts.insert(f.into(), json!(1));
            total += 1;
        }
    }
    (total, counts)
}

/// Étiquettes d'une fiche (tableau, ou chaîne séparée par virgules / retours).
fn sheet_tags(data: &Value) -> Vec<String> {
    match data.get("tags") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim().to_string())
            .collect(),
        Some(Value::String(s)) => s.split([',', '\n']).map(|t| t.trim().to_string()).collect(),
        _ => vec![],
    }
    .into_iter()
    .filter(|t| !t.is_empty())
    .collect()
}

/// Membres d'une faction (fiche tag) : ce qu'elle cite + ce qui la cite.
fn faction_member_ids(sheets: &[Value], tag: &Value) -> BTreeSet<String> {
    let tag_id = tag["_id"].as_str().unwrap_or("");
    let tag_uuid = journal_uuid(tag_id);
    let mut out = BTreeSet::new();
    let tdata = get_path(tag, "flags.campaign-codex.data")
        .cloned()
        .unwrap_or(json!({}));
    for f in [
        "associates",
        "linkedLocations",
        "linkedShops",
        "linkedGroups",
    ] {
        for u in tdata.get(f).and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = u.as_str().and_then(journal_id_of_uuid) {
                out.insert(id.to_string());
            }
        }
    }
    for s in sheets {
        let data = get_path(s, "flags.campaign-codex.data")
            .cloned()
            .unwrap_or(json!({}));
        let cites = ["associates", "linkedNPCs"].iter().any(|f| {
            data.get(*f)
                .and_then(Value::as_array)
                .is_some_and(|l| l.iter().any(|v| v.as_str() == Some(&tag_uuid)))
        });
        if cites && let Some(id) = s["_id"].as_str() {
            out.insert(id.to_string());
        }
    }
    out.remove(tag_id);
    out
}

// ------------------------------------------------------------ définitions

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

/// Outils `client_*` de ce module → commande compagnon (fusionnés dans
/// `companion::COMPANION_COMMANDS`).
#[rustfmt::skip]
pub const COMMANDS: &[(&str, &str, Delivery)] = &[
    ("client_cc_convert", "cc_convert", Delivery::Unique),
    ("client_cc_export_obsidian", "cc_export_obsidian", Delivery::Unique),
    ("client_cc_open_toc", "cc_open_toc", Delivery::Unique),
    ("client_cc_open_quest_board", "cc_open_quest_board", Delivery::Unique),
    ("client_cc_open_sheet", "cc_open_sheet", Delivery::Unique),
    ("client_cc_migrate_content", "cc_migrate_content", Delivery::Unique),
    ("client_cc_shop_add_item", "cc_shop_add_item", Delivery::Unique),
];

/// Outils sans effet de bord (annotations + mode FOUNDRY_READONLY).
pub const READ_ONLY: &[&str] = &["cc_list_sheets", "cc_get_sheet", "cc_webhub_status"];

const RELATION_ENUM: [&str; 16] = [
    "associates",
    "npc_location",
    "npc_shop",
    "shop_location",
    "region_location",
    "region_region",
    "tag_group",
    "member",
    "journal",
    "scene",
    "actor",
    // valeurs historiques (1.6), toujours acceptées
    "linkedLocations",
    "linkedShops",
    "linkedNPCs",
    "linkedLocation",
    "linkedActor",
];

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    let sheet = json!({"type":"string","description":"Sheet _id, name or JournalEntry uuid"});
    vec![
        ("cc_list_sheets",
         "List Campaign Codex sheets as a light index: type, tags, linked actor, player visibility and link counts per relation field. Filters: type, name_contains, tag (sheet tag/label, case-insensitive) and faction (a tag/faction sheet name or _id: keeps the sheets that faction lists or that list it).",
         json!({"type":"object","properties":{
            "type":{"type":"string","enum":CC_TYPES},
            "tag":{"type":"string"},"faction":{"type":"string"},
            "name_contains":{"type":"string"}}})),
        ("cc_get_sheet",
         "One Campaign Codex sheet: type, sheet class, image, the Information / GM Notes / custom-tab prose read from its managed pages (CC 6.x content storage, with the legacy data.description fallback), the raw data (links, tags, inventory), the quest object for quest sheets, tab-overrides and whether players can see it.",
         json!({"type":"object","properties":{"_id":{"type":"string"},"name":{"type":"string"}}})),
        ("cc_create_sheet",
         "Create a Campaign Codex sheet exactly as Campaign Codex 6.x does: core sheetClass, contentStorageVersion 1, managed Information and GM Notes pages, per-type default data (a quest sheet gets its default quest object). Hidden from players by default (player_visible:true shows it; legacy gm_only:true also hides it). description fills the Information page, notes the GM Notes page. folder = JournalEntry folder name or _id. linked_actor (npc/tag only) also records the sheet on the actor, like CC.",
         json!({"type":"object","properties":{
            "name":{"type":"string"},"type":{"type":"string","enum":CC_TYPES},
            "description":{"type":"string","description":"HTML for the Information page"},
            "notes":{"type":"string","description":"HTML for the GM Notes page (never shown to players)"},
            "image":{"type":"string"},"linked_actor":{"type":"string"},
            "tags":{"type":"array","items":{"type":"string"}},
            "player_visible":{"type":"boolean"},"gm_only":{"type":"boolean"},
            "folder":{"type":"string"}},"required":["name","type"]})),
        ("cc_link",
         "Link (or unlink with unlink:true) two Campaign Codex endpoints, writing BOTH sides like Campaign Codex does (its own hooks only run in the browser that made the change, so nothing else would mirror it). Endpoints: sheet _id/name/uuid, or Actor./Scene./JournalEntry. uuids. relation is deduced from the types when omitted: npc|tag<->npc|tag associates; npc|tag<->location|region npc_location; npc|tag<->shop npc_shop; tag<->group tag_group; location|region<->shop shop_location (a shop has one location: the previous one loses it); region<->location region_location (a location has one parent region: other regions lose it); region->region region_region (from = parent, cycles refused); group->any sheet member (one-sided); sheet->plain journal journal (linkedStandardJournals); sheet->scene scene (linkedScene + scene.journal); npc|tag->actor actor (linkedActor + the actor's linkedSheets). Legacy relation values (linkedLocations, linkedShops, linkedNPCs, linkedLocation, linkedActor) are still accepted. bidirectional defaults to true; false writes only the 'from' document and skips the cleanup of previous links.",
         json!({"type":"object","properties":{
            "from":{"type":"string"},"to":{"type":"string"},
            "relation":{"type":"string","enum":RELATION_ENUM},
            "unlink":{"type":"boolean"},
            "bidirectional":{"type":"boolean"}},"required":["from","to"]})),
        ("cc_quest_update",
         "Update the quest of a Campaign Codex quest sheet (quests[0]): status (active | inactive | completed | failed; also sets the quest-board column), pinned (shown on the quest board), urgency, giver (sheet or actor name/_id/uuid, \"\" clears), related (list replacing relatedUuids), rewards {xp, currency, reputation}, hide_rewards, check_in (a note added to the check-in log), title, description. Keeps updatedAt and the linkedQuests back-links on the giver/related sheets in sync. notify:true posts a chat message announcing the change. Player visibility of the quest = the sheet's visibility (cc_set_visibility).",
         json!({"type":"object","properties":{
            "sheet":sheet,
            "status":{"type":"string","enum":["active","inactive","completed","failed"]},
            "pinned":{"type":"boolean"},"urgency":{"type":"string","enum":["low","medium","high"]},
            "giver":{"type":"string"},
            "related":{"type":"array","items":{"type":"string"}},
            "rewards":{"type":"object","properties":{"xp":{"type":"number"},"currency":{"type":"number"},"reputation":{"type":"number"}}},
            "hide_rewards":{"type":"boolean"},
            "check_in":{"type":"string"},"title":{"type":"string"},"description":{"type":"string"},
            "notify":{"type":"boolean"}},"required":["sheet"]})),
        ("cc_objective",
         "Edit the objective tree of a Campaign Codex quest sheet. op=add (text, optional parent_id for a sub-objective, completed/failed/visible), op=set (id + any of text/completed/failed/visible; completed and failed are mutually exclusive), op=remove (id, removes its sub-objectives too). New objectives are hidden from players unless visible:true. Returns the updated tree.",
         json!({"type":"object","properties":{
            "sheet":sheet,"op":{"type":"string","enum":["add","set","remove"]},
            "id":{"type":"string"},"parent_id":{"type":"string"},"text":{"type":"string"},
            "completed":{"type":"boolean"},"failed":{"type":"boolean"},"visible":{"type":"boolean"}},
            "required":["sheet","op"]})),
        ("cc_set_visibility",
         "Player visibility of a Campaign Codex sheet. player_visible sets the journal's default ownership (OBSERVER 2 / NONE 0) — for a quest sheet this is what shows it on the players' quest board. tabs [{key, hidden, visible, label}] writes tab-overrides (hidden = GM-only tab, visible = tab shown at all; keys like info, notes, locations, npcs, shops, inventory, associates, quests, journals or custom-info-*). hidden_links (sheet refs) hides those links from players (hiddenAssociates; hidden_links_mode replace | add | remove). Managed content pages are re-aligned (GM Notes always restricted, Information/custom pages restricted when their tab is hidden).",
         json!({"type":"object","properties":{
            "sheet":sheet,"player_visible":{"type":"boolean"},
            "tabs":{"type":"array","items":{"type":"object","properties":{
                "key":{"type":"string"},"hidden":{"type":"boolean"},"visible":{"type":"boolean"},"label":{"type":"string"}},"required":["key"]}},
            "hidden_links":{"type":"array","items":{"type":"string"}},
            "hidden_links_mode":{"type":"string","enum":["replace","add","remove"]}},"required":["sheet"]})),
        ("cc_set_content",
         "Write the prose of a Campaign Codex sheet into its managed page: tab = info (default) | notes (GM only) | a custom tab key (custom-info-*) or label. mode replace (default) | append. A missing managed page is created and indexed; create_tab:true adds a new custom tab (label = tab_label or tab) when none matches. Legacy sheets (content storage 0) get the legacy data field instead — open them once in Foundry, or run client_cc_migrate_content, to migrate.",
         json!({"type":"object","properties":{
            "sheet":sheet,"tab":{"type":"string"},"content":{"type":"string","description":"HTML"},
            "mode":{"type":"string","enum":["replace","append"]},
            "create_tab":{"type":"boolean"},"tab_label":{"type":"string"}},"required":["sheet","content"]})),
        ("cc_shop_set_item",
         "Adjust a Campaign Codex shop (entry) sheet server-side: an existing inventory line (item = itemUuid, item _id or name) gets quantity, custom_price (null = item price), infinite, visible_to_players, or remove:true; markup sets the shop's price multiplier. Adding a NEW item needs the browser (item snapshot, price lookup): use client_cc_shop_add_item.",
         json!({"type":"object","properties":{
            "shop":sheet,"item":{"type":"string"},
            "quantity":{"type":"number"},"custom_price":{"type":["number","null"]},
            "infinite":{"type":"boolean"},"visible_to_players":{"type":"boolean"},
            "remove":{"type":"boolean"},"markup":{"type":"number"}},"required":["shop"]})),
        ("cc_webhub_status",
         "Read-only: Campaign Codex Web Hub Sync state of the sheets (flags of campaign-codex-web-hub-sync): synced (has a cloudId), excluded (syncDisabled), sourceWorldId, synced content keys. Never contacts the Web Hub. Optional sheet or type filter.",
         json!({"type":"object","properties":{
            "sheet":sheet,"type":{"type":"string","enum":CC_TYPES}}})),
        ("cc_webhub_exclude",
         "Exclude (excluded:true, default) or re-include a Campaign Codex sheet from Web Hub Sync by setting/removing its syncDisabled flag. It only changes the flag: it never triggers a sync or contacts the Web Hub.",
         json!({"type":"object","properties":{
            "sheet":sheet,"sheets":{"type":"array","items":{"type":"string"}},
            "excluded":{"type":"boolean"}}})),
        // --- Campaign Codex client-side (compagnon) ---
        ("client_cc_convert",
         "Campaign Codex (companion): convert a plain Journal Entry (uuid) into a NEW Campaign Codex sheet (the original stays). type: location | npc | region | shop | group. pages_to_separate_sheets makes one sheet per text page. Returns the created sheets' uuids.",
         json!({"type":"object","properties":{
            "uuid":{"type":"string"},"type":{"type":"string","enum":CONVERT_TYPES},
            "pages_to_separate_sheets":{"type":"boolean"}},"required":["uuid","type"]})),
        ("client_cc_export_obsidian",
         "Campaign Codex (companion): export the whole codex to a Markdown/Obsidian zip (downloaded in the GM browser).",
         json!({"type":"object","properties":{}})),
        ("client_cc_open_toc",
         "Campaign Codex (companion): open the Campaign Codex Hub on the GM client — never closes it when already open (re-applies the options instead). view_mode: type | label | folder | home; tags filters by sheet tags with tag_mode and | or. Legacy 'tab' is treated as view_mode.",
         json!({"type":"object","properties":{
            "view_mode":{"type":"string","enum":["type","label","folder","home"]},
            "tags":{"type":"array","items":{"type":"string"}},
            "tag_mode":{"type":"string","enum":["and","or"]},
            "tab":{"type":"string"}}})),
        ("client_cc_open_quest_board",
         "Campaign Codex (companion): open the quest board on the GM client (brought to front if already open, never toggled closed).",
         json!({"type":"object","properties":{}})),
        ("client_cc_open_sheet",
         "Campaign Codex (companion): open a Campaign Codex sheet on the GM client and report the sheet class actually used.",
         json!({"type":"object","properties":{"sheet":sheet},"required":["sheet"]})),
        ("client_cc_migrate_content",
         "Campaign Codex (companion, GM): migrate/repair every sheet to the 6.x page content storage (migrateAllJournalContent, silent). Returns {total, migrated, repaired, failed[]}.",
         json!({"type":"object","properties":{}})),
        ("client_cc_shop_add_item",
         "Campaign Codex (companion): add an Item (uuid — world or compendium — or world item _id/name) to a shop sheet's inventory through Campaign Codex itself (proper snapshot, price, category), quantity added to an existing line; then applies custom_price / infinite / visible_to_players to that line.",
         json!({"type":"object","properties":{
            "shop":sheet,"item":{"type":"string"},"quantity":{"type":"number"},
            "custom_price":{"type":["number","null"]},"infinite":{"type":"boolean"},
            "visible_to_players":{"type":"boolean"}},"required":["shop","item"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

// ------------------------------------------------------------ accès Foundry

/// Modules activés (setting core.moduleConfiguration).
async fn module_active(state: &McpState, id: &str) -> Result<bool> {
    let w = json!({"key": "core.moduleConfiguration"})
        .as_object()
        .cloned()
        .unwrap_or_default();
    let setting = state
        .foundry
        .get_documents("settings", Some(&w), None, 0, Some(1))
        .await?
        .into_iter()
        .next();
    let config: Map<String, Value> = setting
        .as_ref()
        .and_then(|s| s.get("value"))
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    Ok(config.get(id).and_then(Value::as_bool).unwrap_or(false))
}

async fn require_cc(state: &McpState) -> Result<()> {
    if !module_active(state, MODULE).await? {
        bail!("The campaign-codex module is not active in this world");
    }
    Ok(())
}

fn strip_journal_prefix(ident: &str) -> &str {
    journal_id_of_uuid(ident).unwrap_or(ident)
}

async fn get_sheet(state: &McpState, ident: &str) -> Result<Option<Value>> {
    let doc = state
        .foundry
        .find_document("journal", strip_journal_prefix(ident), None)
        .await?;
    Ok(doc.filter(|d| cc_type_of(d).is_some()))
}

async fn require_sheet(state: &McpState, ident: &str) -> Result<Value> {
    get_sheet(state, ident)
        .await?
        .ok_or_else(|| anyhow!("No Campaign Codex sheet found: {ident}"))
}

async fn all_sheets(state: &McpState) -> Result<Vec<Value>> {
    let docs = state
        .foundry
        .get_documents("journal", None, None, 0, None)
        .await?;
    Ok(docs
        .into_iter()
        .filter(|d| cc_type_of(d).is_some())
        .collect())
}

fn sheet_arg(args: &Value, key: &str) -> Result<String> {
    str_arg(args, key)
        .or_else(|| str_arg(args, "_id"))
        .or_else(|| str_arg(args, "name"))
        .ok_or_else(|| anyhow!("'{key}' is required"))
}

/// Une extrémité de lien résolue.
struct Endpoint {
    kind: Kind,
    id: String,
    uuid: String,
    name: String,
    doc: Value,
}

/// Résout un identifiant : uuid typé (Actor./Scene./JournalEntry.), sinon
/// journal (_id puis nom), puis acteur, puis scène. `only` force la collection.
async fn resolve_endpoint(state: &McpState, ident: &str, only: Option<&str>) -> Result<Endpoint> {
    let (forced, raw) = if let Some(r) = ident.strip_prefix("Actor.") {
        (Some("actors"), r)
    } else if let Some(r) = ident.strip_prefix("Scene.") {
        (Some("scenes"), r)
    } else if ident.starts_with("JournalEntry.") {
        (Some("journal"), strip_journal_prefix(ident))
    } else {
        (only, ident)
    };
    let order: Vec<&str> = match forced {
        Some(c) => vec![c],
        None => vec!["journal", "actors", "scenes"],
    };
    for coll in order {
        let fields: Option<Vec<String>> = match coll {
            "journal" => None,
            _ => Some(vec![
                "_id".into(),
                "name".into(),
                "flags".into(),
                "journal".into(),
            ]),
        };
        if let Some(doc) = state
            .foundry
            .find_document(coll, raw, fields.as_deref())
            .await?
        {
            let id = doc["_id"].as_str().unwrap_or("").to_string();
            let name = doc["name"].as_str().unwrap_or("").to_string();
            let (kind, uuid) = match coll {
                "actors" => (Kind::Actor, format!("Actor.{id}")),
                "scenes" => (Kind::Scene, format!("Scene.{id}")),
                _ => (
                    cc_type_of(&doc).map(Kind::Cc).unwrap_or(Kind::Journal),
                    journal_uuid(&id),
                ),
            };
            return Ok(Endpoint {
                kind,
                id,
                uuid,
                name,
                doc,
            });
        }
    }
    bail!("Not found: {ident}")
}

async fn update_journals(state: &McpState, updates: Vec<Value>) -> Result<Value> {
    if updates.is_empty() {
        return Ok(Value::Null);
    }
    state
        .foundry
        .modify_document(
            "JournalEntry",
            "update",
            json!({"action": "update", "diff": false, "recursive": true, "render": true, "updates": updates}),
        )
        .await
}

async fn update_pages(state: &McpState, journal_id: &str, updates: Vec<Value>) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    state
        .foundry
        .modify_document(
            "JournalEntryPage",
            "update",
            json!({"action": "update", "parentUuid": journal_uuid(journal_id),
                   "diff": false, "recursive": true, "render": true, "updates": updates}),
        )
        .await?;
    Ok(())
}

/// Enregistre la fiche dans flags.campaign-codex.linkedSheets de l'acteur (ou l'en retire).
async fn sync_actor_linked_sheet(
    state: &McpState,
    actor: &Value,
    sheet_uuid: &str,
    sheet_name: &str,
    unlink: bool,
) -> Result<bool> {
    let mut list: Vec<Value> = get_path(actor, "flags.campaign-codex.linkedSheets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has = list.iter().any(|l| l["uuid"].as_str() == Some(sheet_uuid));
    if unlink {
        if !has {
            return Ok(false);
        }
        list.retain(|l| l["uuid"].as_str() != Some(sheet_uuid));
    } else {
        if has {
            return Ok(false);
        }
        list.push(json!({"name": sheet_name, "uuid": sheet_uuid}));
        list.sort_by_key(|l| l["name"].as_str().unwrap_or("").to_lowercase());
    }
    state
        .foundry
        .modify_document(
            "Actor",
            "update",
            json!({"action": "update", "diff": false, "recursive": true, "render": true,
                   "updates": [{"_id": actor["_id"], "flags": {"campaign-codex": {"linkedSheets": list}}}]}),
        )
        .await?;
    Ok(true)
}

async fn resolve_folder(state: &McpState, ident: &str) -> Result<String> {
    let mut w = Map::new();
    w.insert("type".into(), json!("JournalEntry"));
    let folders = state
        .foundry
        .get_documents("folders", Some(&w), None, 0, None)
        .await?;
    folders
        .iter()
        .find(|f| f["_id"].as_str() == Some(ident))
        .or_else(|| folders.iter().find(|f| f["name"].as_str() == Some(ident)))
        .and_then(|f| f["_id"].as_str().map(String::from))
        .ok_or_else(|| anyhow!("JournalEntry folder not found: {ident}"))
}

fn quest_of(sheet: &Value) -> Result<(Vec<Value>, Value)> {
    if cc_type_of(sheet).as_deref() != Some("quest") {
        bail!(
            "'{}' is not a Campaign Codex quest sheet",
            sheet["name"].as_str().unwrap_or("?")
        );
    }
    let quests = get_path(sheet, "flags.campaign-codex.data.quests")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let q = quests.first().cloned().unwrap_or_else(|| {
        default_quest(
            sheet["name"].as_str().unwrap_or("Quest"),
            &random_id(),
            now_ms(),
        )
    });
    Ok((quests, q))
}

async fn save_quest(
    state: &McpState,
    sheet: &Value,
    mut quests: Vec<Value>,
    q: Value,
) -> Result<()> {
    if quests.is_empty() {
        quests.push(q);
    } else {
        quests[0] = q;
    }
    update_journals(
        state,
        vec![
            json!({"_id": sheet["_id"], "flags": {"campaign-codex": {"data": {"quests": quests}}}}),
        ],
    )
    .await?;
    Ok(())
}

// ------------------------------------------------------------ outils

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "cc_list_sheets" => {
            let sheets = all_sheets(state).await?;
            let type_f = str_arg(args, "type");
            let name_f = str_arg(args, "name_contains").map(|s| s.to_lowercase());
            let tag_f = str_arg(args, "tag").map(|s| s.to_lowercase());
            let faction_ids = match str_arg(args, "faction") {
                Some(f) => {
                    let fid = strip_journal_prefix(&f);
                    let tag = sheets
                        .iter()
                        .filter(|s| cc_type_of(s).as_deref() == Some("tag"))
                        .find(|s| s["_id"].as_str() == Some(fid) || s["name"].as_str() == Some(&f))
                        .ok_or_else(|| anyhow!("No faction (tag) sheet found: {f}"))?;
                    Some(faction_member_ids(&sheets, tag))
                }
                None => None,
            };
            let index: Vec<Value> = sheets
                .iter()
                .filter(|s| type_f.is_none() || cc_type_of(s) == type_f)
                .filter(|s| {
                    name_f.as_ref().is_none_or(|n| {
                        s["name"].as_str().unwrap_or("").to_lowercase().contains(n)
                    })
                })
                .filter(|s| {
                    let data = get_path(s, "flags.campaign-codex.data").cloned().unwrap_or(json!({}));
                    tag_f.as_ref().is_none_or(|t| sheet_tags(&data).iter().any(|x| x.to_lowercase() == *t))
                })
                .filter(|s| {
                    faction_ids
                        .as_ref()
                        .is_none_or(|ids| s["_id"].as_str().is_some_and(|i| ids.contains(i)))
                })
                .map(|s| {
                    let data = get_path(s, "flags.campaign-codex.data").cloned().unwrap_or(json!({}));
                    let (total, counts) = link_counts(&data);
                    json!({
                        "_id": s["_id"], "name": s["name"], "type": cc_type_of(s),
                        "tags": sheet_tags(&data),
                        "linkedActor": data.get("linkedActor").cloned(),
                        "visibleToPlayers": get_path(s, "ownership.default").and_then(Value::as_i64).unwrap_or(0) >= 2,
                        "links": total, "link_counts": counts,
                    })
                })
                .collect();
            Ok(text_response(&Value::Array(index)))
        }

        "cc_get_sheet" => {
            let ident = str_arg(args, "_id")
                .or_else(|| str_arg(args, "name"))
                .ok_or_else(|| anyhow!("Must provide one of: _id or name"))?;
            let sheet = require_sheet(state, &ident).await?;
            let cc = get_path(&sheet, "flags.campaign-codex")
                .cloned()
                .unwrap_or(json!({}));
            let cc_type = cc_type_of(&sheet);
            let custom: Vec<Value> = custom_tabs(&sheet)
                .into_iter()
                .map(|(k, l)| json!({"key": k, "label": l, "content": read_content(&sheet, &k)}))
                .collect();
            let quest = (cc_type.as_deref() == Some("quest"))
                .then(|| get_path(&cc, "data.quests").and_then(|q| q.get(0)).cloned())
                .flatten();
            let mut data = cc.get("data").cloned().unwrap_or(json!({}));
            if let Some(d) = data.as_object_mut() {
                d.remove("quests");
            }
            Ok(text_response(&json!({
                "_id": sheet["_id"], "name": sheet["name"],
                "uuid": journal_uuid(sheet["_id"].as_str().unwrap_or("")),
                "type": cc_type,
                "sheetClass": get_path(&sheet, "flags.core.sheetClass"),
                "image": cc.get("image"),
                "contentStorageVersion": storage_version(&sheet),
                "info": read_content(&sheet, "info"),
                "notes": read_content(&sheet, "notes"),
                "customTabs": custom,
                "quest": quest,
                "tabOverrides": tab_overrides(&sheet),
                "visibleToPlayers": get_path(&sheet, "ownership.default").and_then(Value::as_i64).unwrap_or(0) >= 2,
                "ownership": sheet.get("ownership"),
                "data": data,
            })))
        }

        "cc_create_sheet" => {
            require_cc(state).await?;
            let sheet_name = str_arg(args, "name").ok_or_else(|| anyhow!("'name' is required"))?;
            let cc_type = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            if !CC_TYPES.contains(&cc_type.as_str()) {
                bail!(
                    "Unknown Campaign Codex type '{cc_type}'. Valid: {}",
                    CC_TYPES.join(", ")
                );
            }
            let mut actor = None;
            if let Some(actor_arg) = str_arg(args, "linked_actor") {
                if !NPCISH.contains(&cc_type.as_str()) {
                    bail!("linked_actor only applies to npc and tag sheets");
                }
                actor = Some(resolve_endpoint(state, &actor_arg, Some("actors")).await?);
            }
            let folder = match str_arg(args, "folder") {
                Some(f) => Some(resolve_folder(state, &f).await?),
                None => None,
            };
            let gm_only = args
                .get("gm_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let player_visible = !gm_only
                && args
                    .get("player_visible")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            let description = str_arg(args, "description").unwrap_or_default();
            let notes = str_arg(args, "notes").unwrap_or_default();
            let image = str_arg(args, "image");
            let spec = NewSheet {
                name: &sheet_name,
                cc_type: &cc_type,
                info: &description,
                notes: &notes,
                image: image.as_deref(),
                tags: args.get("tags").filter(|t| t.is_array()).cloned(),
                linked_actor: actor.as_ref().map(|a| a.uuid.clone()),
                player_visible,
                folder,
            };
            let doc = build_sheet_doc(&spec, &random_id(), &random_id(), now_ms());
            let result = foundry
                .modify_document(
                    "JournalEntry",
                    "create",
                    json!({"action": "create", "renderSheet": false, "keepId": false, "data": [doc]}),
                )
                .await?;
            let created_id = result
                .pointer("/result/0/_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(a) = &actor
                && !created_id.is_empty()
            {
                sync_actor_linked_sheet(
                    state,
                    &a.doc,
                    &journal_uuid(&created_id),
                    &sheet_name,
                    false,
                )
                .await?;
            }
            Ok(text_response(&json!({
                "created": {"_id": created_id, "uuid": journal_uuid(&created_id), "name": sheet_name,
                            "type": cc_type, "player_visible": player_visible,
                            "linkedActor": actor.as_ref().map(|a| a.uuid.clone())},
            })))
        }

        "cc_link" => {
            require_cc(state).await?;
            let from_arg = str_arg(args, "from").ok_or_else(|| anyhow!("'from' is required"))?;
            let to_arg = str_arg(args, "to").ok_or_else(|| anyhow!("'to' is required"))?;
            let explicit = Rel::parse(str_arg(args, "relation").as_deref())?;
            let unlink = args.get("unlink").and_then(Value::as_bool).unwrap_or(false);
            let both = args
                .get("bidirectional")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let to_only = match explicit {
                Some(Rel::Actor) => Some("actors"),
                Some(Rel::Scene) => Some("scenes"),
                Some(Rel::Journal) => Some("journal"),
                _ => None,
            };
            let from = resolve_endpoint(state, &from_arg, Some("journal")).await?;
            let to = resolve_endpoint(state, &to_arg, to_only).await?;
            if from.uuid == to.uuid {
                bail!("Cannot link a sheet to itself");
            }
            let (rel, swapped) = resolve_rel(explicit, &from.kind, &to.kind)?;
            let (a, b) = if swapped { (&to, &from) } else { (&from, &to) };
            let (a_side, b_side) = rel.sides();
            let write_a = both || !swapped;
            let write_b = both || swapped;

            let mut store = Store::default();
            store.insert(a.doc.clone());
            if matches!(b.kind, Kind::Cc(_)) {
                store.insert(b.doc.clone());
            }
            let mut extra = vec![];
            // garde-fous et nettoyages propres à certaines relations
            if !unlink && both {
                match rel {
                    Rel::RegionRegion | Rel::RegionLocation | Rel::Member => {
                        let sheets = all_sheets(state).await?;
                        let map: HashMap<String, Value> = sheets
                            .into_iter()
                            .filter_map(|s| s["_id"].as_str().map(String::from).map(|i| (i, s)))
                            .collect();
                        if rel == Rel::RegionRegion && region_ancestors(&map, &a.id).contains(&b.id)
                        {
                            bail!(
                                "'{}' is already an ancestor of '{}': linking would create a region cycle",
                                b.name,
                                a.name
                            );
                        }
                        if rel == Rel::Member
                            && b.kind == Kind::Cc("group".into())
                            && (nested_groups(&map, &b.id).contains(&a.id))
                        {
                            bail!(
                                "'{}' already contains '{}': circular group membership",
                                b.name,
                                a.name
                            );
                        }
                        if rel == Rel::RegionLocation {
                            for (id, s) in map.iter() {
                                if *id != a.id
                                    && cc_type_of(s).as_deref() == Some("region")
                                    && get_path(s, "flags.campaign-codex.data.linkedLocations")
                                        .and_then(Value::as_array)
                                        .is_some_and(|l| {
                                            l.iter().any(|v| v.as_str() == Some(&b.uuid))
                                        })
                                {
                                    store.insert(s.clone());
                                    store.remove(id, "linkedLocations", &b.uuid);
                                    extra.push(json!({"removed_from_region": s["name"]}));
                                }
                            }
                        }
                    }
                    Rel::ShopLocation => {
                        if let Some(old) = store
                            .field(&b.id, "linkedLocation")
                            .and_then(Value::as_str)
                            .map(String::from)
                            && old != a.uuid
                            && let Some(old_id) = journal_id_of_uuid(&old)
                            && let Some(old_doc) = get_sheet(state, old_id).await?
                        {
                            store.insert(old_doc);
                            if store.remove(old_id, "linkedShops", &b.uuid) {
                                extra.push(
                                    json!({"removed_from_previous_location": store.name(old_id)}),
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
            let mut changed = vec![];
            if write_a
                && let Some(side) = a_side
                && store.link(&a.id, side, &b.uuid, unlink)
            {
                changed.push(json!({"sheet": a.name, "field": side.0}));
            }
            if write_b
                && let Some(side) = b_side
                && store.link(&b.id, side, &a.uuid, unlink)
            {
                changed.push(json!({"sheet": b.name, "field": side.0}));
            }
            let result = update_journals(state, store.updates()).await?;
            // côtés hors flags CC
            if write_b {
                match rel {
                    Rel::Scene => {
                        let cur = b.doc.get("journal").and_then(Value::as_str);
                        let want = if unlink {
                            (cur == Some(&a.id)).then_some(Value::Null)
                        } else {
                            (cur != Some(&a.id)).then(|| json!(a.id))
                        };
                        if let Some(v) = want {
                            foundry
                                .modify_document(
                                    "Scene",
                                    "update",
                                    json!({"action": "update", "diff": false, "recursive": true, "render": true,
                                           "updates": [{"_id": b.id, "journal": v}]}),
                                )
                                .await?;
                            changed.push(json!({"scene": b.name, "field": "journal"}));
                        }
                    }
                    Rel::Actor
                        if sync_actor_linked_sheet(state, &b.doc, &a.uuid, &a.name, unlink)
                            .await? =>
                    {
                        changed.push(
                            json!({"actor": b.name, "field": "flags.campaign-codex.linkedSheets"}),
                        );
                    }
                    _ => {}
                }
            }
            Ok(text_response(&json!({
                "from": from.name, "to": to.name, "relation": rel.name(),
                "action": if unlink { "unlink" } else { "link" },
                "bidirectional": both,
                "changed": changed, "cleanup": extra,
                "unchanged": changed.is_empty(),
                "result": if result.is_null() { Value::Null } else { json!("ok") },
            })))
        }

        "cc_quest_update" => {
            require_cc(state).await?;
            let sheet = require_sheet(state, &sheet_arg(args, "sheet")?).await?;
            let sheet_id = sheet["_id"].as_str().unwrap_or("").to_string();
            let (quests, mut q) = quest_of(&sheet)?;
            let before = q.clone();
            if let Some(s) = str_arg(args, "status") {
                apply_quest_status(&mut q, &s)?;
            }
            if let Some(b) = args.get("pinned").and_then(Value::as_bool) {
                q["pinned"] = json!(b);
            }
            if let Some(u) = str_arg(args, "urgency") {
                if !["low", "medium", "high"].contains(&u.as_str()) {
                    bail!("urgency must be low | medium | high");
                }
                q["urgency"] = json!(u);
            }
            if let Some(t) = str_arg(args, "title") {
                q["title"] = json!(t);
            }
            if let Some(d) = str_arg(args, "description") {
                q["description"] = json!(d);
            }
            if let Some(b) = args.get("hide_rewards").and_then(Value::as_bool) {
                q["hideRewards"] = json!(b);
            }
            if let Some(r) = args.get("rewards").and_then(Value::as_object) {
                for (k, field) in [
                    ("xp", "rewardXP"),
                    ("currency", "rewardCurrency"),
                    ("reputation", "rewardReputation"),
                ] {
                    if let Some(n) = r.get(k).filter(|v| v.is_number()) {
                        q[field] = n.clone();
                    }
                }
            }
            if let Some(g) = str_arg(args, "giver") {
                if g.is_empty() {
                    q["questGiverUuid"] = json!("");
                } else {
                    let e = resolve_endpoint(state, &g, None).await?;
                    if !matches!(e.kind, Kind::Cc(_) | Kind::Actor) {
                        bail!("The quest giver must be a Campaign Codex sheet or an actor");
                    }
                    if e.id == sheet_id {
                        bail!("A quest cannot be its own giver");
                    }
                    q["questGiverUuid"] = json!(e.uuid);
                }
            }
            if let Some(list) = args.get("related").and_then(Value::as_array) {
                let mut uuids = vec![];
                for r in list.iter().filter_map(Value::as_str) {
                    let e = resolve_endpoint(state, r, None).await?;
                    if e.id == sheet_id {
                        continue;
                    }
                    if q["questGiverUuid"].as_str() == Some(&e.uuid) {
                        bail!(
                            "'{}' is already the quest giver: a sheet cannot be both giver and related",
                            e.name
                        );
                    }
                    if !uuids.contains(&json!(e.uuid)) {
                        uuids.push(json!(e.uuid));
                    }
                }
                q["relatedUuids"] = Value::Array(uuids);
            }
            if let Some(note) = str_arg(args, "check_in").filter(|n| !n.trim().is_empty()) {
                let by = match foundry.user_id().await {
                    Some(uid) => foundry
                        .find_document("users", &uid, Some(&["_id".into(), "name".into()]))
                        .await?
                        .and_then(|u| u["name"].as_str().map(String::from))
                        .unwrap_or_else(|| "Gamemaster".into()),
                    None => "Gamemaster".into(),
                };
                if !q["checkIns"].is_array() {
                    q["checkIns"] = json!([]);
                }
                q["checkIns"]
                    .as_array_mut()
                    .expect("tableau")
                    .insert(0, json!({"note": note, "by": by, "at": now_ms()}));
            }
            q["updatedAt"] = json!(now_ms());
            save_quest(state, &sheet, quests, q.clone()).await?;

            // linkedQuests inverses sur les fiches donneuses / liées
            let (old, new) = (
                quest_backlink_ids(&before, &sheet_id),
                quest_backlink_ids(&q, &sheet_id),
            );
            let self_uuid = journal_uuid(&sheet_id);
            let mut store = Store::default();
            let mut backlinks = vec![];
            for id in old.symmetric_difference(&new) {
                if let Some(doc) = get_sheet(state, id).await? {
                    store.insert(doc);
                    let added = new.contains(id);
                    let did = if added {
                        store.add(id, "linkedQuests", &self_uuid)
                    } else {
                        store.remove(id, "linkedQuests", &self_uuid)
                    };
                    if did {
                        backlinks.push(json!({"sheet": store.name(id), "linkedQuests": if added { "added" } else { "removed" }}));
                    }
                }
            }
            update_journals(state, store.updates()).await?;

            let mut chat = Value::Null;
            if args.get("notify").and_then(Value::as_bool).unwrap_or(false) {
                let title = q["title"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .or(sheet["name"].as_str())
                    .unwrap_or("Quest");
                let mut html = format!(
                    "<div class=\"campaign-codex-quest-update\"><h3>{}</h3><p><strong>Status:</strong> {}</p>",
                    html_escape(title),
                    quest_status_label(&q)
                );
                if let Some(note) = str_arg(args, "check_in") {
                    html.push_str(&format!("<p>{}</p>", html_escape(&note)));
                }
                html.push_str("</div>");
                post_chat(state, &html, json!({}), None).await?;
                chat = json!("posted");
            }
            Ok(text_response(&json!({
                "sheet": sheet["name"], "status": quest_status_label(&q), "quest": q,
                "backlinks": backlinks, "chat": chat,
            })))
        }

        "cc_objective" => {
            require_cc(state).await?;
            let sheet = require_sheet(state, &sheet_arg(args, "sheet")?).await?;
            let (quests, mut q) = quest_of(&sheet)?;
            let mut objs: Vec<Value> = q["objectives"].as_array().cloned().unwrap_or_default();
            let id = objective_op(&mut objs, args, &random_id())?;
            q["objectives"] = Value::Array(objs.clone());
            q["updatedAt"] = json!(now_ms());
            save_quest(state, &sheet, quests, q).await?;
            Ok(text_response(&json!({
                "sheet": sheet["name"], "op": str_arg(args, "op"), "id": id, "objectives": objs,
            })))
        }

        "cc_set_visibility" => {
            require_cc(state).await?;
            let sheet = require_sheet(state, &sheet_arg(args, "sheet")?).await?;
            let sheet_id = sheet["_id"].as_str().unwrap_or("").to_string();
            let mut update = json!({"_id": sheet_id});
            let mut report = Map::new();
            if let Some(v) = args.get("player_visible").and_then(Value::as_bool) {
                update["ownership"] = json!({"default": if v { 2 } else { 0 }});
                report.insert("player_visible".into(), json!(v));
            }
            let mut overrides = tab_overrides(&sheet);
            if let Some(tabs) = args.get("tabs").and_then(Value::as_array) {
                for t in tabs {
                    let key = t
                        .get("key")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("each tab needs a key"))?;
                    let pos = overrides
                        .iter()
                        .position(|o| o.get("key").and_then(Value::as_str) == Some(key));
                    let entry = match pos {
                        Some(i) => &mut overrides[i],
                        None => {
                            overrides.push(json!({"key": key}));
                            overrides.last_mut().expect("élément")
                        }
                    };
                    for k in ["hidden", "visible"] {
                        if let Some(b) = t.get(k).and_then(Value::as_bool) {
                            entry[k] = json!(b);
                        }
                    }
                    if let Some(l) = t.get("label").and_then(Value::as_str) {
                        entry["label"] = json!(l);
                    }
                }
                set_nested(
                    &mut update,
                    "flags.campaign-codex.tab-overrides",
                    Value::Array(overrides.clone()),
                );
                report.insert("tab_overrides".into(), Value::Array(overrides.clone()));
            }
            if let Some(list) = args.get("hidden_links").and_then(Value::as_array) {
                let mode = str_arg(args, "hidden_links_mode").unwrap_or_else(|| "replace".into());
                let mut hidden: Vec<Value> = if mode == "replace" {
                    vec![]
                } else {
                    get_path(&sheet, "flags.campaign-codex.data.hiddenAssociates")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                };
                for r in list.iter().filter_map(Value::as_str) {
                    let e = resolve_endpoint(state, r, None).await?;
                    let v = json!(e.uuid);
                    match mode.as_str() {
                        "remove" => hidden.retain(|h| *h != v),
                        "replace" | "add" => {
                            if !hidden.contains(&v) {
                                hidden.push(v)
                            }
                        }
                        other => {
                            bail!("hidden_links_mode must be replace | add | remove, not {other}")
                        }
                    }
                }
                set_nested(
                    &mut update,
                    "flags.campaign-codex.data.hiddenAssociates",
                    Value::Array(hidden.clone()),
                );
                report.insert("hiddenAssociates".into(), Value::Array(hidden));
            }
            if update.as_object().is_some_and(|o| o.len() > 1) {
                update_journals(state, vec![update]).await?;
            }
            let page_updates = page_ownership_updates(&sheet, &overrides);
            report.insert("pages_realigned".into(), json!(page_updates.len()));
            update_pages(state, &sheet_id, page_updates).await?;
            Ok(text_response(
                &json!({"sheet": sheet["name"], "changes": report}),
            ))
        }

        "cc_set_content" => {
            require_cc(state).await?;
            let sheet = require_sheet(state, &sheet_arg(args, "sheet")?).await?;
            let sheet_id = sheet["_id"].as_str().unwrap_or("").to_string();
            let content =
                str_arg(args, "content").ok_or_else(|| anyhow!("'content' is required"))?;
            let append = match str_arg(args, "mode").as_deref() {
                None | Some("replace") => false,
                Some("append") => true,
                Some(o) => bail!("mode must be replace | append, not {o}"),
            };
            let tab = str_arg(args, "tab").unwrap_or_else(|| "info".into());
            let tabs = custom_tabs(&sheet);
            let low = tab.trim().to_lowercase();
            let mut new_tab: Option<(String, String)> = None;
            let (key, label) = if low == "info" || low == "information" {
                ("info".to_string(), "Information".to_string())
            } else if low == "notes" || low == "gm notes" || low == "gm_notes" {
                ("notes".to_string(), "GM Notes".to_string())
            } else if let Some((k, l)) = tabs
                .iter()
                .find(|(k, l)| *k == low || l.to_lowercase() == low)
            {
                (k.clone(), l.clone())
            } else if args
                .get("create_tab")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let mut n = tabs.len() + 1;
                while tabs
                    .iter()
                    .any(|(k, _)| *k == format!("{CUSTOM_PREFIX}{n}"))
                {
                    n += 1;
                }
                let l = str_arg(args, "tab_label").unwrap_or_else(|| tab.clone());
                let t = (format!("{CUSTOM_PREFIX}{n}"), l);
                new_tab = Some(t.clone());
                t
            } else {
                bail!(
                    "Unknown tab '{tab}'. Use info, notes, or one of the custom tabs {:?} (create_tab:true adds a new one)",
                    tabs.iter()
                        .map(|(k, l)| format!("{k} ({l})"))
                        .collect::<Vec<_>>()
                );
            };
            let current = read_content(&sheet, &key);
            let next = if append && !current.is_empty() {
                format!("{current}{content}")
            } else {
                content
            };

            let mut sheet_update = json!({"_id": sheet_id});
            if let Some((k, l)) = &new_tab {
                let mut raw = get_path(&sheet, "flags.campaign-codex.custom-info-tabs")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                raw.push(
                    json!({"key": k, "label": l, "icon": "fas fa-info-circle", "isCustom": true}),
                );
                set_nested(
                    &mut sheet_update,
                    "flags.campaign-codex.custom-info-tabs",
                    Value::Array(raw),
                );
            }
            let mut created_page: Option<String> = None;
            let storage = if storage_version(&sheet) >= 1 {
                if let Some(page) = find_content_page(&sheet, &key) {
                    update_pages(
                        state,
                        &sheet_id,
                        vec![json!({"_id": page["_id"], "text": {"content": next}})],
                    )
                    .await?;
                    "page"
                } else {
                    // page gérée absente : on la crée et on l'indexe
                    let pid = random_id();
                    let n_custom = tabs.len() + usize::from(new_tab.is_some());
                    let sort = match key.as_str() {
                        "info" => 100000,
                        "notes" => (n_custom as i64 + 2) * 100000,
                        k => {
                            let idx = tabs
                                .iter()
                                .position(|(tk, _)| tk == k)
                                .unwrap_or(tabs.len());
                            (idx as i64 + 2) * 100000
                        }
                    };
                    let restricted = key_restricted(&tab_overrides(&sheet), &key);
                    let page = content_page(&key, &label, sort, restricted, &next, &pid);
                    let created = foundry
                        .modify_document(
                            "JournalEntryPage",
                            "create",
                            json!({"action": "create", "parentUuid": journal_uuid(&sheet_id),
                                   "keepId": true, "renderSheet": false, "data": [page]}),
                        )
                        .await?;
                    // le serveur ajoute l'auteur (OWNER) à l'ownership de la page :
                    // CC veut exactement {default} et la « réparerait » sinon.
                    // « ==ownership » = remplacement forcé (accepté en v13 et v14).
                    let has_extra = created
                        .pointer("/result/0/ownership")
                        .and_then(Value::as_object)
                        .is_some_and(|o| o.keys().any(|k| k != "default"));
                    if has_extra {
                        update_pages(
                            state,
                            &sheet_id,
                            vec![json!({"_id": pid, "==ownership": {"default": if restricted { 0 } else { -1 }}})],
                        )
                        .await?;
                    }
                    created_page = Some(pid.clone());
                    // la page GM Notes reste en dernier
                    if new_tab.is_some()
                        && let Some(np) = find_content_page(&sheet, "notes")
                    {
                        update_pages(
                            state,
                            &sheet_id,
                            vec![json!({"_id": np["_id"], "sort": (n_custom as i64 + 2) * 100000})],
                        )
                        .await?;
                    }
                    "page (created)"
                }
            } else {
                let lp =
                    legacy_path(&key).ok_or_else(|| anyhow!("Unsupported content key {key}"))?;
                set_nested(
                    &mut sheet_update,
                    &format!("flags.campaign-codex.data.{lp}"),
                    json!(next),
                );
                "legacy data field (content storage 0)"
            };
            if sheet_update.as_object().is_some_and(|o| o.len() > 1) {
                update_journals(state, vec![sheet_update]).await?;
            }
            if let Some(pid) = created_page {
                // index contentPages dans l'ordre info, onglets custom, notes
                let mut order: Vec<String> = vec!["info".into()];
                order.extend(tabs.iter().map(|(k, _)| k.clone()));
                if let Some((k, _)) = &new_tab {
                    order.push(k.clone());
                }
                order.push("notes".into());
                let ordered: Vec<(String, String)> = order
                    .into_iter()
                    .filter_map(|k| {
                        let id = if k == key {
                            Some(pid.clone())
                        } else {
                            find_content_page(&sheet, &k)
                                .and_then(|p| p["_id"].as_str())
                                .map(String::from)
                        };
                        id.map(|i| (k, i))
                    })
                    .collect();
                let current: Vec<String> = get_path(&sheet, "flags.campaign-codex.contentPages")
                    .and_then(Value::as_object)
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                let patches = content_index_patches(&current, &ordered);
                for patch in patches {
                    update_journals(
                        state,
                        vec![json!({"_id": sheet_id, "flags": {"campaign-codex": {"contentPages": patch}}})],
                    )
                    .await?;
                }
            }
            Ok(text_response(&json!({
                "sheet": sheet["name"], "tab": key, "label": label,
                "mode": if append { "append" } else { "replace" },
                "stored_in": storage, "length": next.len(),
            })))
        }

        "cc_shop_set_item" => {
            require_cc(state).await?;
            let sheet = require_sheet(state, &sheet_arg(args, "shop")?).await?;
            if cc_type_of(&sheet).as_deref() != Some("shop") {
                bail!(
                    "'{}' is not a Campaign Codex shop (entry) sheet",
                    sheet["name"].as_str().unwrap_or("?")
                );
            }
            let mut data_upd = Map::new();
            let mut report = Map::new();
            if let Some(m) = args.get("markup").filter(|v| v.is_number()) {
                data_upd.insert("markup".into(), m.clone());
                report.insert("markup".into(), m.clone());
            }
            if let Some(item) = str_arg(args, "item") {
                let mut inv = get_path(&sheet, "flags.campaign-codex.data.inventory")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let i = find_inventory_row(&inv, &item).ok_or_else(|| {
                    anyhow!("No inventory line matches '{item}' in this shop (use client_cc_shop_add_item to add a new item)")
                })?;
                if args.get("remove").and_then(Value::as_bool).unwrap_or(false) {
                    let row = inv.remove(i);
                    report.insert("removed".into(), row["name"].clone());
                } else {
                    let row = &mut inv[i];
                    if let Some(q) = args.get("quantity").and_then(Value::as_f64) {
                        row["quantity"] = json!(q.max(0.0).floor() as i64);
                    }
                    if let Some(p) = args.get("custom_price") {
                        if p.is_null() || p.is_number() {
                            row["customPrice"] = p.clone();
                        }
                    }
                    if let Some(b) = args.get("infinite").and_then(Value::as_bool) {
                        row["infinite"] = json!(b);
                    }
                    if let Some(b) = args.get("visible_to_players").and_then(Value::as_bool) {
                        row["visibleToPlayers"] = json!(b);
                    }
                    report.insert("line".into(), row.clone());
                }
                data_upd.insert("inventory".into(), Value::Array(inv));
            }
            if data_upd.is_empty() {
                bail!("Nothing to change: give item (+ settings) and/or markup");
            }
            update_journals(
                state,
                vec![json!({"_id": sheet["_id"], "flags": {"campaign-codex": {"data": data_upd}}})],
            )
            .await?;
            Ok(text_response(
                &json!({"shop": sheet["name"], "changes": report}),
            ))
        }

        "cc_webhub_status" => {
            let active = module_active(state, WEBHUB).await?;
            let sheets = match str_arg(args, "sheet") {
                Some(s) => vec![require_sheet(state, &s).await?],
                None => all_sheets(state).await?,
            };
            let type_f = str_arg(args, "type");
            let rows: Vec<Value> = sheets
                .iter()
                .filter(|s| type_f.is_none() || cc_type_of(s) == type_f)
                .map(|s| {
                    let f = get_path(s, &format!("flags.{WEBHUB}"))
                        .cloned()
                        .unwrap_or(json!({}));
                    let cloud = f
                        .get("cloudId")
                        .and_then(Value::as_str)
                        .filter(|c| !c.is_empty());
                    let keys: Vec<String> = f
                        .get("syncState")
                        .and_then(Value::as_object)
                        .map(|m| m.keys().cloned().collect())
                        .unwrap_or_default();
                    json!({
                        "_id": s["_id"], "name": s["name"], "type": cc_type_of(s),
                        "synced": cloud.is_some(),
                        "excluded": f.get("syncDisabled").and_then(Value::as_bool).unwrap_or(false),
                        "cloudId": cloud, "sourceWorldId": f.get("sourceWorldId"),
                        "syncedContentKeys": keys,
                    })
                })
                .collect();
            let synced = rows.iter().filter(|r| r["synced"] == json!(true)).count();
            let excluded = rows.iter().filter(|r| r["excluded"] == json!(true)).count();
            Ok(text_response(&json!({
                "webhub_module_active": active,
                "summary": {"sheets": rows.len(), "synced": synced, "excluded": excluded},
                "sheets": rows,
            })))
        }

        "cc_webhub_exclude" => {
            let mut idents: Vec<String> = args
                .get("sheets")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            if let Some(s) = str_arg(args, "sheet") {
                idents.push(s);
            }
            if idents.is_empty() {
                bail!("Give 'sheet' or 'sheets'");
            }
            let excluded = args
                .get("excluded")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let mut updates = vec![];
            let mut names = vec![];
            for i in &idents {
                let s = require_sheet(state, i).await?;
                let flag = if excluded {
                    json!({"syncDisabled": true})
                } else {
                    json!({"-=syncDisabled": null})
                };
                updates.push(json!({"_id": s["_id"], "flags": {WEBHUB: flag}}));
                names.push(s["name"].clone());
            }
            update_journals(state, updates).await?;
            let active = module_active(state, WEBHUB).await?;
            Ok(text_response(&json!({
                "sheets": names, "excluded": excluded,
                "webhub_module_active": active,
                "note": "Flag only — no synchronisation was triggered.",
            })))
        }

        // ---------------------------------------------------- Campaign Codex (compagnon)
        "client_cc_convert" => {
            let uuid = str_arg(args, "uuid").ok_or_else(|| anyhow!("'uuid' is required"))?;
            let cc_type = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            if !CONVERT_TYPES.contains(&cc_type.as_str()) {
                bail!(
                    "Campaign Codex can only convert to: {}",
                    CONVERT_TYPES.join(", ")
                );
            }
            let pages = args
                .get("pages_to_separate_sheets")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let r = call_companion(
                state,
                cmd_for(name),
                json!({ "uuid": uuid, "type": cc_type, "pagesToSeparateSheets": pages }),
                None,
                30,
            )
            .await?;
            Ok(text_response(&r))
        }
        "client_cc_export_obsidian" => {
            let r = call_companion(state, cmd_for(name), json!({}), None, 60).await?;
            Ok(text_response(&r))
        }
        "client_cc_open_toc" => {
            let mut a = json!({});
            if let Some(v) = str_arg(args, "view_mode").or_else(|| str_arg(args, "tab")) {
                a["viewMode"] = json!(v);
            }
            if let Some(t) = args.get("tags").filter(|t| t.is_array()) {
                a["tags"] = t.clone();
            }
            if let Some(m) = str_arg(args, "tag_mode") {
                a["tagMode"] = json!(m);
            }
            let r = call_companion(state, cmd_for(name), a, None, 10).await?;
            Ok(text_response(&r))
        }
        "client_cc_open_quest_board" | "client_cc_migrate_content" => {
            let timeout = if name == "client_cc_migrate_content" {
                120
            } else {
                10
            };
            let r = call_companion(state, cmd_for(name), json!({}), None, timeout).await?;
            Ok(text_response(&r))
        }
        "client_cc_open_sheet" => {
            let sheet = require_sheet(state, &sheet_arg(args, "sheet")?).await?;
            let uuid = journal_uuid(sheet["_id"].as_str().unwrap_or(""));
            let r = call_companion(state, cmd_for(name), json!({"uuid": uuid}), None, 15).await?;
            Ok(text_response(&r))
        }
        "client_cc_shop_add_item" => {
            let sheet = require_sheet(state, &sheet_arg(args, "shop")?).await?;
            if cc_type_of(&sheet).as_deref() != Some("shop") {
                bail!(
                    "'{}' is not a Campaign Codex shop (entry) sheet",
                    sheet["name"].as_str().unwrap_or("?")
                );
            }
            let item = str_arg(args, "item").ok_or_else(|| anyhow!("'item' is required"))?;
            let item_uuid = if item.contains('.') {
                item
            } else {
                let doc = foundry
                    .find_document("items", &item, Some(&["_id".into(), "name".into()]))
                    .await?
                    .ok_or_else(|| {
                        anyhow!("World item not found: {item} (give a compendium uuid otherwise)")
                    })?;
                format!("Item.{}", doc["_id"].as_str().unwrap_or(""))
            };
            let mut a = json!({
                "shopUuid": journal_uuid(sheet["_id"].as_str().unwrap_or("")),
                "itemUuid": item_uuid,
                "quantity": args.get("quantity").and_then(Value::as_f64).unwrap_or(1.0),
            });
            for (k, js) in [
                ("custom_price", "customPrice"),
                ("infinite", "infinite"),
                ("visible_to_players", "visibleToPlayers"),
            ] {
                if let Some(v) = args.get(k) {
                    a[js] = v.clone();
                }
            }
            let r = call_companion(state, cmd_for(name), a, None, 20).await?;
            Ok(text_response(&r))
        }

        other => bail!("Unknown tool: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(t: &str) -> Kind {
        Kind::Cc(t.into())
    }

    #[test]
    fn random_id_format() {
        let id = random_id();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(id, random_id());
    }

    #[test]
    fn sheet_doc_v6() {
        let s = NewSheet {
            name: "[CC] Bar",
            cc_type: "location",
            info: "<p>i</p>",
            notes: "<p>n</p>",
            image: None,
            tags: Some(json!(["port"])),
            linked_actor: None,
            player_visible: false,
            folder: Some("F1".into()),
        };
        let d = build_sheet_doc(&s, "AAAAAAAAAAAAAAAA", "BBBBBBBBBBBBBBBB", 1);
        assert_eq!(
            d["flags"]["core"]["sheetClass"],
            "campaign-codex.LocationSheet"
        );
        assert_eq!(d["flags"]["campaign-codex"]["contentStorageVersion"], 1);
        assert_eq!(
            d["flags"]["campaign-codex"]["contentPages"]["notes"],
            "BBBBBBBBBBBBBBBB"
        );
        assert_eq!(d["ownership"]["default"], 0);
        assert_eq!(d["folder"], "F1");
        assert!(
            d["flags"]["campaign-codex"]["data"]
                .get("widgets")
                .is_none()
        );
        assert_eq!(
            d["flags"]["campaign-codex"]["data"]["tags"],
            json!(["port"])
        );
        let pages = d["pages"].as_array().unwrap();
        assert_eq!(pages[0]["ownership"]["default"], -1);
        assert_eq!(pages[1]["ownership"]["default"], 0);
        assert_eq!(
            pages[1]["flags"]["core"]["sheetClass"],
            CONTENT_PAGE_SHEET_CLASS
        );
        assert_eq!(read_content(&d, "info"), "<p>i</p>");
        assert_eq!(read_content(&d, "notes"), "<p>n</p>");
    }

    #[test]
    fn quest_sheet_has_default_quest() {
        let s = NewSheet {
            name: "Q",
            cc_type: "quest",
            info: "",
            notes: "",
            image: None,
            tags: None,
            linked_actor: None,
            player_visible: true,
            folder: None,
        };
        let d = build_sheet_doc(&s, "a", "b", 7);
        let q = &d["flags"]["campaign-codex"]["data"]["quests"][0];
        assert_eq!(q["title"], "Q");
        assert_eq!(q["inactive"], true);
        assert_eq!(q["updatedAt"], 7);
        assert_eq!(d["ownership"]["default"], 2);
        assert!(
            d["flags"]["campaign-codex"]["data"]
                .get("associates")
                .is_none()
        );
        assert_eq!(cc_type_of(&d).as_deref(), Some("quest"));
    }

    #[test]
    fn legacy_content_fallback() {
        let d = json!({"flags": {"campaign-codex": {"type": "npc", "data": {"description": "old", "notes": ["n"]}}}});
        assert_eq!(read_content(&d, "info"), "old");
        assert_eq!(read_content(&d, "notes"), "n");
        let v1 = json!({"flags": {"campaign-codex": {"type": "npc", "contentStorageVersion": 1, "data": {"description": "old"}}}, "pages": []});
        assert_eq!(read_content(&v1, "info"), "");
    }

    #[test]
    fn content_page_by_key_when_index_stale() {
        let d = json!({
            "flags": {"campaign-codex": {"contentStorageVersion": 1, "contentPages": {"info": "zzz"}}},
            "pages": [{"_id": "p1", "text": {"content": "hello"}, "flags": {"campaign-codex": {"contentKey": "info"}}}],
        });
        assert_eq!(read_content(&d, "info"), "hello");
    }

    #[test]
    fn type_from_sheet_class() {
        let d = json!({"flags": {"core": {"sheetClass": "campaign-codex.NPCSheet"}}});
        assert_eq!(cc_type_of(&d).as_deref(), Some("npc"));
        assert_eq!(cc_type_of(&json!({"flags": {}})), None);
    }

    #[test]
    fn relation_deduction() {
        assert_eq!(
            resolve_rel(None, &cc("npc"), &cc("tag")).unwrap(),
            (Rel::Associates, false)
        );
        assert_eq!(
            resolve_rel(None, &cc("location"), &cc("npc")).unwrap(),
            (Rel::NpcLocation, true)
        );
        assert_eq!(
            resolve_rel(None, &cc("shop"), &cc("region")).unwrap(),
            (Rel::ShopLocation, true)
        );
        assert_eq!(
            resolve_rel(None, &cc("region"), &cc("location")).unwrap(),
            (Rel::RegionLocation, false)
        );
        assert_eq!(
            resolve_rel(None, &cc("region"), &cc("region")).unwrap(),
            (Rel::RegionRegion, false)
        );
        assert_eq!(
            resolve_rel(None, &cc("group"), &cc("tag")).unwrap(),
            (Rel::TagGroup, true)
        );
        assert_eq!(
            resolve_rel(None, &cc("npc"), &cc("group")).unwrap(),
            (Rel::Member, true)
        );
        assert_eq!(
            resolve_rel(None, &cc("quest"), &Kind::Journal).unwrap(),
            (Rel::Journal, false)
        );
        assert_eq!(
            resolve_rel(None, &cc("npc"), &Kind::Actor).unwrap(),
            (Rel::Actor, false)
        );
        assert!(resolve_rel(None, &cc("location"), &cc("location")).is_err());
        assert!(resolve_rel(None, &cc("location"), &Kind::Actor).is_err());
        // relation explicite incompatible
        assert!(resolve_rel(Some(Rel::NpcShop), &cc("npc"), &cc("location")).is_err());
        // group -> tag forcé en member
        assert_eq!(
            resolve_rel(Some(Rel::Member), &cc("group"), &cc("tag")).unwrap(),
            (Rel::Member, false)
        );
    }

    #[test]
    fn legacy_relation_names() {
        assert_eq!(Rel::parse(Some("linkedActor")).unwrap(), Some(Rel::Actor));
        assert_eq!(
            Rel::parse(Some("linkedLocation")).unwrap(),
            Some(Rel::ShopLocation)
        );
        assert_eq!(Rel::parse(Some("linkedNPCs")).unwrap(), None);
        assert_eq!(
            Rel::parse(Some("associates")).unwrap(),
            Some(Rel::Associates)
        );
        assert!(Rel::parse(Some("bogus")).is_err());
    }

    #[test]
    fn store_writes_both_sides() {
        let mut st = Store::default();
        st.insert(json!({"_id": "n", "name": "N", "flags": {"campaign-codex": {"data": {"linkedLocations": []}}}}));
        st.insert(json!({"_id": "l", "name": "L", "flags": {"campaign-codex": {"data": {}}}}));
        let (a, b) = Rel::NpcLocation.sides();
        assert!(st.link("n", a.unwrap(), "JournalEntry.l", false));
        assert!(st.link("l", b.unwrap(), "JournalEntry.n", false));
        assert!(!st.link("n", a.unwrap(), "JournalEntry.l", false));
        let ups = st.updates();
        assert_eq!(ups.len(), 2);
        assert_eq!(st.list("l", "linkedNPCs"), vec![json!("JournalEntry.n")]);
        assert!(st.link("n", a.unwrap(), "JournalEntry.l", true));
        assert!(st.list("n", "linkedLocations").is_empty());
        // scalaire : clear_if seulement si c'est la bonne cible
        st.insert(json!({"_id": "s", "flags": {"campaign-codex": {"data": {"linkedLocation": "JournalEntry.x"}}}}));
        assert!(!st.clear_if("s", "linkedLocation", "JournalEntry.y"));
        assert!(st.clear_if("s", "linkedLocation", "JournalEntry.x"));
    }

    #[test]
    fn region_cycle_detection() {
        let mut m = HashMap::new();
        m.insert(
            "a".to_string(),
            json!({"flags": {"campaign-codex": {"data": {"parentRegions": ["JournalEntry.b"]}}}}),
        );
        m.insert(
            "b".to_string(),
            json!({"flags": {"campaign-codex": {"data": {"parentRegions": ["JournalEntry.c"]}}}}),
        );
        m.insert(
            "c".to_string(),
            json!({"flags": {"campaign-codex": {"data": {}}}}),
        );
        let anc = region_ancestors(&m, "a");
        assert!(anc.contains("b") && anc.contains("c"));
        assert!(region_ancestors(&m, "c").is_empty());
    }

    #[test]
    fn quest_status_rules() {
        let mut q = default_quest("Q", "x", 0);
        apply_quest_status(&mut q, "completed").unwrap();
        assert_eq!(
            (
                q["completed"].as_bool(),
                q["failed"].as_bool(),
                q["inactive"].as_bool()
            ),
            (Some(true), Some(false), Some(false))
        );
        assert_eq!(q["boardColumn"], "completed");
        apply_quest_status(&mut q, "failed").unwrap();
        assert_eq!(q["completed"], false);
        assert_eq!(q["boardColumn"], "failed");
        apply_quest_status(&mut q, "active").unwrap();
        assert_eq!(quest_status_label(&q), "active");
        apply_quest_status(&mut q, "inactive").unwrap();
        assert_eq!(quest_status_label(&q), "inactive");
        assert!(apply_quest_status(&mut q, "done").is_err());
    }

    #[test]
    fn quest_backlinks_only_journals() {
        let q = json!({"questGiverUuid": "Actor.a", "relatedUuids": ["JournalEntry.j1", "JournalEntry.self", "JournalEntry.j2.JournalEntryPage.p"]});
        let ids = quest_backlink_ids(&q, "self");
        assert_eq!(
            ids.into_iter().collect::<Vec<_>>(),
            vec!["j1".to_string(), "j2".to_string()]
        );
    }

    #[test]
    fn objective_tree_ops() {
        let mut objs = vec![];
        objective_op(&mut objs, &json!({"op": "add", "text": "A"}), "o1").unwrap();
        objective_op(
            &mut objs,
            &json!({"op": "add", "text": "A.1", "parent_id": "o1", "visible": true}),
            "o2",
        )
        .unwrap();
        assert_eq!(objs[0]["objectives"][0]["text"], "A.1");
        assert_eq!(objs[0]["objectives"][0]["visible"], true);
        objective_op(
            &mut objs,
            &json!({"op": "set", "id": "o2", "completed": true}),
            "-",
        )
        .unwrap();
        assert_eq!(objs[0]["objectives"][0]["completed"], true);
        objective_op(
            &mut objs,
            &json!({"op": "set", "id": "o2", "failed": true}),
            "-",
        )
        .unwrap();
        assert_eq!(objs[0]["objectives"][0]["completed"], false);
        assert!(objective_op(&mut objs, &json!({"op": "set", "id": "nope"}), "-").is_err());
        assert!(objective_op(&mut objs, &json!({"op": "add", "parent_id": "nope"}), "o3").is_err());
        objective_op(&mut objs, &json!({"op": "remove", "id": "o2"}), "-").unwrap();
        assert!(objs[0]["objectives"].as_array().unwrap().is_empty());
        objective_op(&mut objs, &json!({"op": "remove", "id": "o1"}), "-").unwrap();
        assert!(objs.is_empty());
    }

    #[test]
    fn page_ownership_alignment() {
        let d = json!({"pages": [
            {"_id": "i", "ownership": {"default": -1}, "flags": {"campaign-codex": {"contentKey": "info"}}},
            {"_id": "n", "ownership": {"default": -1}, "flags": {"campaign-codex": {"contentKey": "notes"}}},
            {"_id": "x", "ownership": {"default": 0}},
        ]});
        let ups = page_ownership_updates(&d, &[]);
        assert_eq!(
            ups,
            vec![json!({"_id": "n", "==ownership": {"default": 0}})]
        );
        let ups = page_ownership_updates(&d, &[json!({"key": "info", "hidden": true})]);
        assert_eq!(ups.len(), 2);
    }

    #[test]
    fn content_index_order() {
        let p = content_index_patches(
            &["info".into(), "notes".into()],
            &[
                ("info".into(), "a".into()),
                ("custom-info-1".into(), "c".into()),
                ("notes".into(), "b".into()),
            ],
        );
        assert_eq!(p.len(), 4);
        assert_eq!(p[0], json!({"-=info": null, "-=notes": null}));
        assert_eq!(p[1], json!({"info": "a"}));
        assert_eq!(p[3], json!({"notes": "b"}));
    }

    #[test]
    fn inventory_row_lookup() {
        let inv = vec![
            json!({"itemUuid": "Item.abc", "itemId": "abc", "name": "Rope"}),
            json!({"itemUuid": "Compendium.x.y.Item.def", "itemId": "def", "name": "Torch"}),
        ];
        assert_eq!(find_inventory_row(&inv, "Item.abc"), Some(0));
        assert_eq!(find_inventory_row(&inv, "def"), Some(1));
        assert_eq!(find_inventory_row(&inv, "torch"), Some(1));
        assert_eq!(find_inventory_row(&inv, "Sword"), None);
    }

    #[test]
    fn list_counts_and_tags() {
        let data = json!({"associates": ["a", "b"], "linkedQuests": ["q"], "linkedActor": "Actor.x", "linkedScene": null, "tags": "Port, Docks"});
        let (total, counts) = link_counts(&data);
        assert_eq!(total, 4);
        assert_eq!(counts["associates"], 2);
        assert_eq!(sheet_tags(&data), vec!["Port", "Docks"]);
    }

    #[test]
    fn faction_membership() {
        let tag = json!({"_id": "t", "flags": {"campaign-codex": {"type": "tag", "data": {"associates": ["JournalEntry.n1"], "linkedLocations": ["JournalEntry.l1"]}}}});
        let n2 = json!({"_id": "n2", "flags": {"campaign-codex": {"type": "npc", "data": {"associates": ["JournalEntry.t"]}}}});
        let other = json!({"_id": "o", "flags": {"campaign-codex": {"type": "npc", "data": {}}}});
        let ids = faction_member_ids(&[tag.clone(), n2, other], &tag);
        assert_eq!(ids.into_iter().collect::<Vec<_>>(), vec!["l1", "n1", "n2"]);
    }

    #[test]
    fn custom_tab_keys() {
        let d = json!({"flags": {"campaign-codex": {"custom-info-tabs": [{"key": "custom-info-1", "label": "Lore"}, {"key": "Secrets!"}, {"key": "custom-info-1"}]}}});
        let t = custom_tabs(&d);
        assert_eq!(
            t,
            vec![
                ("custom-info-1".to_string(), "Lore".to_string()),
                ("custom-info-secrets".to_string(), "Custom 2".to_string())
            ]
        );
    }
}
