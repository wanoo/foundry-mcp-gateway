//! Mini Calendar (mc_*) — temps (core.time), notes (journaux), météo,
//! configuration du calendrier + compagnon.
//!
//! Modèle de données de Mini Calendar 10.x (relevé dans le code du module) :
//!   · notes : journaux « Calendar Events - Mini Calendar » (MJ) et « Player
//!     Notes - Mini Calendar » (joueurs, propriété par défaut OWNER), rangés
//!     dans le dossier « Mini Calendar ». Une page par jour nommée
//!     `YYYY-MM-DD` (mois/jour en base 1) ; les récurrentes sur la page
//!     « 0000-Recurring ». La vérité est `flags["wgtgm-mini-calendar"].notes`,
//!     le `text.content` n'est qu'un rendu HTML régénéré à chaque écriture ;
//!   · météo : réglage monde `weatherHistoryStore`, clé `"Y-M-D"` en base 0 ;
//!     températures en °F ;
//!   · calendrier : `calendarSource` ("world" | "custom") +
//!     `calendarConfiguration` (format CalendarData de Foundry + extensions
//!     seasons/weather/sun/moons).

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::foundry::client::FoundryHandle;
use crate::mcp::McpState;
use crate::tools::companion::{Delivery, call_companion, cmd_for};
use crate::tools::{ToolDef, carry, str_arg, text_response};

const MODULE: &str = "wgtgm-mini-calendar";
const EVENTS_JOURNAL: &str = "Calendar Events - Mini Calendar";
const PLAYER_JOURNAL: &str = "Player Notes - Mini Calendar";
const NOTES_FOLDER: &str = "Mini Calendar";
const RECURRING_PAGE: &str = "0000-Recurring";

/// Journaux où Mini Calendar range ses notes (relevés sur un monde réel).
const MC_NOTE_JOURNALS: [(&str, &str); 2] =
    [("player", PLAYER_JOURNAL), ("events", EVENTS_JOURNAL)];

/// Icônes proposées par l'éditeur de notes (helper.js, PIN_TYPES).
#[rustfmt::skip]
const PIN_TYPES: &[(&str, &str)] = &[
    ("fas fa-book", "Note"), ("fas fa-map-pin", "Pin"), ("fas fa-scroll", "Quest"),
    ("fas fa-skull-crossbones", "Danger"), ("fas fa-gem", "Treasure"), ("fas fa-beer", "Tavern"),
    ("fas fa-home", "Village"), ("fas fa-user", "NPC"), ("fas fa-store", "Shop"),
    ("fas fa-sun", "Sun"), ("fas fa-moon", "Moon/Eclipse"), ("fas fa-star", "Star"),
    ("fas fa-snowflake", "Snow"), ("fas fa-glass-cheers", "Festival"), ("fas fa-tree", "Nature"),
    ("fas fa-wheat", "Harvest"), ("fas fa-leaf", "Leaf"), ("fas fa-balance-scale", "Balance"),
    ("fas fa-campground", "Camp"), ("fas fa-chess-rook", "Castle"),
    ("fas fa-place-of-worship", "Temple"), ("fas fa-dungeon", "Dungeon"),
    ("fas fa-dragon", "Monster"), ("fas fa-hand-holding-magic", "Magic"),
    ("fas fa-fist-raised", "Battle"), ("fas fa-shield-alt", "Shield"),
    ("fas fa-water", "Water/Sea"), ("fas fa-anchor", "Port"),
];

/// Types météo intégrés (weather-data.js, BUILTIN_WEATHER_TYPES) :
/// (id, libellé, icône, décalage de température en °F).
#[rustfmt::skip]
const BUILTIN_WEATHER: &[(&str, &str, &str, i64)] = &[
    ("none", "Clear", "fas fa-sun", 14),
    ("partlyCloudy", "Scattered Clouds", "fas fa-cloud-sun", 7),
    ("clouds", "Overcast", "fas fa-cloud", 0),
    ("lightRain", "Light Rain", "fas fa-cloud-rain", -7),
    ("rain", "Rain", "fas fa-cloud-showers-heavy", -14),
    ("heavyRain", "Heavy Rain", "fas fa-cloud-showers-heavy", -14),
    ("thunderstorm", "Thunderstorm", "fas fa-cloud-bolt", -14),
    ("lightThunder", "Thunderstorm (light)", "fas fa-cloud-bolt", -7),
    ("dryThunder", "Thunderstorm (dry)", "fas fa-cloud-bolt", 0),
    ("rainStorm", "Storm", "fas fa-bolt", -14),
    ("fog", "Fog", "fas fa-smog", -7),
    ("lightSnow", "Snow", "fas fa-snowflake", -7),
    ("snow", "Heavy Snow", "fas fa-snowflake", -14),
    ("blizzard", "Blizzard", "fas fa-snow-blowing", -14),
    ("leaves", "Windy", "fas fa-wind", -7),
    ("sandstorm", "Sandstorm", "fas fa-wind", -14),
    ("hail", "Hail", "fas fa-cloud-hail", -7),
    ("aurora", "Aurora", "fas fa-moon-over-sun", -7),
    ("heatWave", "Heatwave", "fas fa-sun-haze", 21),
    ("lightWind", "Light Wind", "fas fa-wind", -7),
    ("heavyWind", "Strong Wind", "fas fa-windsock", -7),
];

/// Calendrier par défaut du cœur Foundry (« Simplified Gregorian ») — celui
/// qu'utilise Mini Calendar quand `calendarSource` vaut "world" et que le
/// système ne fournit pas le sien. Le serveur ne peut pas lire
/// `CONFIG.time.worldCalendarConfig` : c'est une hypothèse, signalée en sortie.
static DEFAULT_CALENDAR: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    let months = [
        ("January", 31, None),
        ("February", 28, Some(29)),
        ("March", 31, None),
        ("April", 30, None),
        ("May", 31, None),
        ("June", 30, None),
        ("July", 31, None),
        ("August", 31, None),
        ("September", 30, None),
        ("October", 31, None),
        ("November", 30, None),
        ("December", 31, None),
    ];
    let months: Vec<Value> = months
        .iter()
        .enumerate()
        .map(|(i, (n, d, l))| {
            let mut m = json!({"name": n, "ordinal": i + 1, "days": d});
            if let Some(l) = l {
                m["leapDays"] = json!(l);
            }
            m
        })
        .collect();
    let days: Vec<Value> = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| json!({"name": n, "ordinal": i + 1}))
    .collect();
    json!({
        "name": "Simplified Gregorian",
        "years": {"yearZero": 0, "firstWeekday": 0, "leapYear": {"leapStart": 8, "leapInterval": 4}},
        "months": {"values": months},
        "days": {"values": days, "daysPerYear": 365, "hoursPerDay": 24, "minutesPerHour": 60, "secondsPerMinute": 60},
        "seasons": {"values": [
            {"name": "Spring", "monthStart": 3, "monthEnd": 5},
            {"name": "Summer", "monthStart": 6, "monthEnd": 8},
            {"name": "Fall", "monthStart": 9, "monthEnd": 11},
            {"name": "Winter", "monthStart": 12, "monthEnd": 2},
        ]},
    })
});

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

/// Outils `client_*` de ce module → commande compagnon (fusionnés dans
/// `companion::COMPANION_COMMANDS`).
pub const COMMANDS: &[(&str, &str, Delivery)] = &[
    ("client_mc_set_time", "mc_set_time", Delivery::Unique),
    ("client_mc_open", "mc_open", Delivery::Unique),
    ("client_mc_get_date", "mc_get_date", Delivery::Unique),
    ("client_mc_set_date", "mc_set_date", Delivery::Unique),
    ("client_mc_set_weather", "mc_set_weather", Delivery::Unique),
];

/// Outils sans effet de bord (annotations + mode FOUNDRY_READONLY).
pub const READ_ONLY: &[&str] = &[
    "mc_get_time",
    "mc_list_notes",
    "mc_get_weather",
    "mc_weather_types",
    "mc_calendar_info",
    "client_mc_get_date",
];

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    let date_schema = json!({"description":"Calendar date, month and day 1-based: \"YYYY-MM-DD\" (e.g. \"1492-03-07\") or {year, month, day}.",
        "oneOf":[{"type":"string"},{"type":"object","properties":{"year":{"type":"integer"},"month":{"type":"integer"},"day":{"type":"integer"}},"required":["year","month","day"]}]});
    let links_schema = json!({"type":"array","description":"Linked documents: UUID strings (\"Actor.abc\", \"JournalEntry.xyz\", \"Compendium.pack.Item.id\") or {type, uuid}. Replaces the existing list on update.",
        "items":{"oneOf":[{"type":"string"},{"type":"object","properties":{"type":{"type":"string"},"uuid":{"type":"string"}},"required":["uuid"]}]}});
    let note_props = |required_date: bool| {
        let mut p = json!({
            "title":{"type":"string"},
            "content":{"type":"string","description":"Note body (HTML or plain text)."},
            "icon":{"type":"string","description":"Font Awesome class (\"fas fa-scroll\") or a Mini Calendar pin label: Note, Pin, Quest, Danger, Treasure, Tavern, Village, NPC, Shop, Sun, Moon/Eclipse, Star, Snow, Festival, Nature, Harvest, Leaf, Balance, Camp, Castle, Temple, Dungeon, Monster, Magic, Battle, Shield, Water/Sea, Port. Default Note (fas fa-book)."},
            "hour":{"type":"integer","description":"Start hour; omit (or all_day:true) for an all-day note."},
            "minute":{"type":"integer"},
            "all_day":{"type":"boolean","description":"true clears hour/minute."},
            "duration":{"type":"integer","minimum":1,"description":"Length in days (multi-day span). Default 1."},
            "player_visible":{"type":"boolean","description":"GM notes: shown to players too. Default false."},
            "linked_documents": links_schema,
            "linked_macros":{"type":"array","items":{"type":"string"},"description":"Macro ids or UUIDs run from the note."},
            "auto_execute_macros":{"type":"boolean"},
        });
        p["date"] = date_schema.clone();
        if !required_date {
            p["date"]["description"] = json!("Move the note to this date (month/day 1-based).");
        }
        p
    };
    let mut add_props = note_props(true);
    add_props["journal"] = json!({"type":"string","enum":["events","player"],"description":"events (GM journal, default) or player (player notes journal)."});
    add_props["user"] = json!({"type":"string","description":"Player notes: owning user (name or id). Default: the gateway's user."});
    let mut upd_props = note_props(false);
    upd_props["id"] = json!({"type":"string","description":"Note id (from mc_list_notes)."});
    vec![
        // --- Mini Calendar ---
        ("mc_get_time",
         "Mini Calendar / core: read the world time (core.time, in seconds) and the matching calendar date. The date is computed server-side with Mini Calendar's arithmetic from its custom calendar; when calendarSource is \"world\" the core Simplified Gregorian is assumed (see calendarAssumed) — use client_mc_get_date for the live, localized date.",
         json!({"type":"object","properties":{}})),
        ("mc_set_time",
         "Mini Calendar / core: set the world time. Give world_time (absolute seconds) or advance_seconds (delta). The calendar updates on the world-time change.",
         json!({"type":"object","properties":{
            "world_time":{"type":"number"},"advance_seconds":{"type":"number"}}})),
        ("mc_list_notes",
         "Mini Calendar: list calendar notes, read from the note flags (the source of truth, not the rendered page HTML). which: events (GM journal) | player (player notes) | both (default). from/to bound the dates (inclusive, \"YYYY-MM-DD\" or {year,month,day}, 1-based). One entry per note: date, journal, id, title, icon, content, hour/minute (null = all day), duration (days), playerVisible, owner, links. Recurring notes (page 0000-Recurring) are listed apart under `recurring` with their rule (include_recurring:false to skip); their occurrences are not expanded.",
         json!({"type":"object","properties":{
            "which":{"type":"string","enum":["player","events","both"]},
            "from": date_schema.clone(), "to": date_schema.clone(),
            "include_recurring":{"type":"boolean","description":"Default true."}}})),
        ("mc_add_note",
         "Mini Calendar: add a one-off note on a date, written the way the module does (day page YYYY-MM-DD in the events or player journal, notes in flags, page HTML regenerated). Creates the folder/journal/page if missing; the open calendar picks it up through its journal hooks. Recurring notes are not supported here — create them in the calendar UI. Returns the note with its id.",
         json!({"type":"object","properties": add_props, "required":["date","title"]})),
        ("mc_update_note",
         "Mini Calendar: edit a one-off note by id (fields left out are kept). date moves it to another day page (the old page is deleted when emptied). Recurring notes cannot be edited here (only deleted).",
         json!({"type":"object","properties": upd_props, "required":["id"]})),
        ("mc_delete_note",
         "Mini Calendar: delete a note by id from either journal (the day page is deleted when it becomes empty). Also removes recurring notes from the 0000-Recurring page.",
         json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]})),
        ("mc_get_weather",
         "Mini Calendar: read the stored weather (setting weatherHistoryStore) for `days` consecutive days starting at `from` (default: today, computed as in mc_get_time). Each day gives type, label, temperature in °F and °C, morning/evening variations and whether it was set manually, or generated:false when the module has not generated that day yet (it generates today + 5 days when the GM's calendar runs). Visual weather effects are client-side.",
         json!({"type":"object","properties":{
            "from": date_schema.clone(),
            "days":{"type":"integer","minimum":1,"maximum":60,"description":"Default 6."}}})),
        ("mc_weather_types",
         "Mini Calendar: list the weather type ids usable with client_mc_set_weather — the built-in ones and the world's custom types (setting customWeatherTypes) — with label, icon and temperature offset (°F), plus the current biome and temperature unit.",
         json!({"type":"object","properties":{}})),
        ("mc_calendar_info",
         "Mini Calendar: describe the calendar — source (world/custom), months (days, leap days, intercalary), weekdays, time units, leap rule, seasons, weather seasons, sun (dawn/dusk per month), moons (cycle, phases), biome, temperature unit — and today's date. With calendarSource \"world\" the module uses Foundry's/the system's calendar, which the server cannot read: the core Simplified Gregorian is shown as an assumption; call client_mc_get_date for the live one.",
         json!({"type":"object","properties":{}})),
        ("client_mc_set_time",
         "Mini Calendar (companion): set the time of day on the GM client. day = day offset from today (0 = today, 1 = tomorrow, -1 = yesterday); hour (+ minute) = target time, or mode = dawn | dusk | noon | midnight (dawn/dusk from the calendar's sun table). next:true moves to the next day when that time has already passed today (e.g. \"next dawn\"). Without next the time can go backwards (dawn of today when it is noon). world_time (absolute seconds) and advance_seconds (delta) are still accepted.",
         json!({"type":"object","properties":{
            "day":{"type":"integer"},"hour":{"type":"number"},"minute":{"type":"integer"},
            "mode":{"type":"string","enum":["dawn","dusk","noon","midnight"]},
            "next":{"type":"boolean"},
            "world_time":{"type":"number"},"advance_seconds":{"type":"number"}}})),
        ("client_mc_open",
         "Mini Calendar (companion): open the calendar window on the GM client. toggle:true closes it when it is already open (the module's default behaviour); default false = always leaves it open.",
         json!({"type":"object","properties":{"toggle":{"type":"boolean"}}})),
        ("client_mc_get_date",
         "Mini Calendar (companion): the live current date from the GM client's calendar — year, month and day (1-based) with localized month/weekday names, time, season, dawn/dusk, moons with their current phase, and the module's forecast for today. Authoritative when the calendar comes from the world/system.",
         json!({"type":"object","properties":{}})),
        ("client_mc_set_date",
         "Mini Calendar (companion): jump to a calendar date (month and day 1-based). hour/minute default to the current time of day. Can go backwards.",
         json!({"type":"object","properties":{
            "year":{"type":"integer"},"month":{"type":"integer"},"day":{"type":"integer"},
            "hour":{"type":"integer"},"minute":{"type":"integer"}},"required":["year","month","day"]})),
        ("client_mc_set_weather",
         "Mini Calendar (companion): force the weather of today or of a later day (day_delta ≥ 0) through the module API (overrideWeather). type = a weather id from mc_weather_types (e.g. rain, snow, none). temp in unit C (default) or F; when omitted the day's stored temperature is kept. Returns the stored entry (readable with mc_get_weather).",
         json!({"type":"object","properties":{
            "type":{"type":"string"},"temp":{"type":"number"},
            "unit":{"type":"string","enum":["C","F"]},
            "day_delta":{"type":"integer","minimum":0}},"required":["type"]})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

// ------------------------------------------------------------------ réglages

/// Valeur d'un réglage monde, décodée du JSON où Foundry la stocke. `None`
/// si le réglage n'a jamais été écrit (le module utilise alors son défaut).
pub(crate) async fn read_setting(foundry: &FoundryHandle, key: &str) -> Result<Option<Value>> {
    let w = json!({"key": key}).as_object().cloned().unwrap();
    let docs = foundry
        .get_documents("settings", Some(&w), None, 0, Some(1))
        .await?;
    Ok(docs.first().and_then(|d| d.get("value")).map(|v| match v {
        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| v.clone()),
        other => other.clone(),
    }))
}

/// Écrit un réglage monde (update ou create), sérialisé comme Foundry.
pub(crate) async fn write_setting(
    foundry: &FoundryHandle,
    key: &str,
    value: &Value,
) -> Result<Value> {
    let w = json!({"key": key}).as_object().cloned().unwrap();
    let docs = foundry
        .get_documents(
            "settings",
            Some(&w),
            Some(&["_id".into(), "key".into()]),
            0,
            Some(1),
        )
        .await?;
    let serialized = value.to_string();
    match docs.first() {
        Some(doc) => {
            foundry
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
            foundry
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

/// Erreur explicite si le module n'est pas activé dans le monde.
pub(crate) async fn ensure_module_active(
    foundry: &FoundryHandle,
    id: &str,
    label: &str,
) -> Result<()> {
    let conf = read_setting(foundry, "core.moduleConfiguration").await?;
    let active = conf
        .as_ref()
        .and_then(|c| c.get(id))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !active {
        bail!("{label} ({id}) is not active in this world");
    }
    Ok(())
}

async fn world_time(foundry: &FoundryHandle) -> Result<f64> {
    Ok(match read_setting(foundry, "core.time").await? {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    })
}

// --------------------------------------------------------------- calendrier

/// Calendrier en vigueur côté serveur : (source, config, supposé ?).
async fn load_calendar(foundry: &FoundryHandle) -> Result<(String, Value, bool)> {
    let source = read_setting(foundry, &format!("{MODULE}.calendarSource"))
        .await?
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "world".into());
    if source != "world"
        && let Some(cfg) = read_setting(foundry, &format!("{MODULE}.calendarConfiguration")).await?
        && cfg.get("months").is_some()
    {
        return Ok((source, cfg, false));
    }
    Ok((source, DEFAULT_CALENDAR.clone(), true))
}

/// Composantes d'une date (mois/jour en base 0, comme Mini Calendar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Comps {
    pub year: i64,
    pub month: usize,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    pub day_of_week: i64,
}

struct Conf {
    id: String,
    year_zero: i64,
    leap_interval: i64,
    /// (jours, jours bissextiles, intercalaire)
    months: Vec<(i64, i64, bool)>,
    s_per_day: i64,
    s_per_min: i64,
    m_per_hour: i64,
    reset_weekdays: bool,
    first_weekday: i64,
    days_in_week: i64,
}

impl Conf {
    fn from(cfg: &Value) -> Result<Self> {
        let num = |p: &str, d: i64| {
            cfg.pointer(p)
                .and_then(Value::as_f64)
                .map(|f| f as i64)
                .filter(|n| *n != 0)
                .unwrap_or(d)
        };
        let months: Vec<(i64, i64, bool)> = cfg
            .pointer("/months/values")
            .and_then(Value::as_array)
            .map(|ms| {
                ms.iter()
                    .map(|m| {
                        let d = m.get("days").and_then(Value::as_i64).unwrap_or(0);
                        let l = m.get("leapDays").and_then(Value::as_i64).unwrap_or(d);
                        (
                            d,
                            l,
                            m.get("intercalary")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        if months.is_empty() || months.iter().all(|m| m.0 <= 0) {
            bail!("calendar configuration has no months");
        }
        let s_per_min = num("/days/secondsPerMinute", 60);
        let m_per_hour = num("/days/minutesPerHour", 60);
        Ok(Conf {
            id: cfg
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            year_zero: cfg
                .pointer("/years/yearZero")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            leap_interval: cfg
                .pointer("/years/leapYear/leapInterval")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            months,
            s_per_day: s_per_min * m_per_hour * num("/days/hoursPerDay", 24),
            s_per_min,
            m_per_hour,
            reset_weekdays: cfg
                .pointer("/years/resetWeekdays")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            first_weekday: cfg
                .pointer("/years/firstWeekday")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            days_in_week: cfg
                .pointer("/days/values")
                .and_then(Value::as_array)
                .map(|a| a.len() as i64)
                .filter(|n| *n > 0)
                .unwrap_or(7),
        })
    }

    /// Règle bissextile de Mini Calendar (CalendarClass.js) : année multiple
    /// de leapInterval — sauf le préréglage grégorien, en règle complète.
    fn is_leap(&self, year: i64) -> bool {
        if self.id == "gregorian-preset" {
            return (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        }
        self.leap_interval > 0 && year % self.leap_interval == 0
    }

    fn month_days(&self, i: usize, leap: bool) -> i64 {
        let (d, l, _) = self.months[i];
        if leap { l } else { d }
    }

    fn year_days(&self, leap: bool) -> (i64, i64) {
        let mut total = 0;
        let mut inter = 0;
        for (i, m) in self.months.iter().enumerate() {
            let d = self.month_days(i, leap);
            total += d;
            if m.2 {
                inter += d;
            }
        }
        (total, inter)
    }

    /// Portage de MiniCalendarClass.timeToComponents (hors synchro PF2e).
    fn time_to_components(&self, time: i64) -> Comps {
        let mut seconds = time.max(0);
        let mut skipped = 0;
        let mut year = self.year_zero;
        let (std_days, std_inter) = self.year_days(false);
        let (leap_days, leap_inter) = self.year_days(true);
        // Grands cycles d'abord (évite de boucler année par année).
        let (cycle_years, cycle_days, cycle_inter) = if self.id == "gregorian-preset" {
            (
                400,
                303 * std_days + 97 * leap_days,
                303 * std_inter + 97 * leap_inter,
            )
        } else if self.leap_interval > 0 {
            let n = self.leap_interval;
            (
                n,
                (n - 1) * std_days + leap_days,
                (n - 1) * std_inter + leap_inter,
            )
        } else {
            (1, std_days, std_inter)
        };
        let cycle_secs = cycle_days * self.s_per_day;
        if cycle_secs > 0 && seconds >= cycle_secs {
            let cycles = seconds / cycle_secs;
            seconds -= cycles * cycle_secs;
            year += cycles * cycle_years;
            skipped += cycles * cycle_inter;
        }
        loop {
            let (d, inter) = self.year_days(self.is_leap(year));
            let secs = d * self.s_per_day;
            if secs > 0 && seconds >= secs {
                seconds -= secs;
                skipped += inter;
                year += 1;
            } else {
                break;
            }
        }
        let leap = self.is_leap(year);
        let mut month = 0;
        for i in 0..self.months.len() {
            let secs = self.month_days(i, leap) * self.s_per_day;
            if seconds >= secs {
                seconds -= secs;
                if self.months[i].2 {
                    skipped += self.month_days(i, leap);
                }
                month = (i + 1).min(self.months.len() - 1);
            } else {
                month = i;
                break;
            }
        }
        let day = seconds / self.s_per_day;
        seconds -= day * self.s_per_day;
        let hour = seconds / (self.s_per_min * self.m_per_hour);
        seconds -= hour * self.s_per_min * self.m_per_hour;
        let minute = seconds / self.s_per_min;
        seconds -= minute * self.s_per_min;
        let total_days = time.max(0) / self.s_per_day;
        let day_of_week = if self.reset_weekdays {
            day % self.days_in_week
        } else {
            (total_days - skipped + self.first_weekday).rem_euclid(self.days_in_week)
        };
        Comps {
            year,
            month,
            day,
            hour,
            minute,
            second: seconds,
            day_of_week,
        }
    }

    /// Le jour suivant (mois/année reportés).
    fn next_day(&self, (y, m, d): (i64, usize, i64)) -> (i64, usize, i64) {
        let leap = self.is_leap(y);
        if d + 1 < self.month_days(m, leap) {
            (y, m, d + 1)
        } else if m + 1 < self.months.len() {
            (y, m + 1, 0)
        } else {
            (y + 1, 0, 0)
        }
    }
}

/// Nom lisible : une clé i18n (« CALENDAR.GREGORIAN.January ») est réduite à
/// son dernier segment ; un nom libre est rendu tel quel.
fn display_name(raw: &str) -> String {
    let looks_i18n =
        raw.contains('.') && !raw.contains(' ') && raw.split('.').all(|s| !s.is_empty());
    if looks_i18n {
        raw.rsplit('.').next().unwrap_or(raw).to_string()
    } else {
        raw.to_string()
    }
}

/// Saison (par ordinal de mois, avec bouclage fin d'année) — même règle
/// que getForecast du module.
fn season_for(entries: Option<&Value>, ordinal: i64) -> Option<String> {
    entries?.as_array()?.iter().find_map(|s| {
        let start = s.get("monthStart")?.as_i64()?;
        let end = s.get("monthEnd")?.as_i64()?;
        let hit = if start <= end {
            ordinal >= start && ordinal <= end
        } else {
            ordinal >= start || ordinal <= end
        };
        hit.then(|| display_name(s.get("name").and_then(Value::as_str).unwrap_or("")))
    })
}

/// Aube/crépuscule du mois (table `sun`), défaut 6 h / 18 h.
fn sun_for(cfg: &Value, ordinal: i64) -> (f64, f64) {
    cfg.pointer("/sun/values")
        .and_then(Value::as_array)
        .and_then(|vs| {
            vs.iter().find(|v| {
                let s = v
                    .get("monthStart")
                    .and_then(Value::as_i64)
                    .unwrap_or(i64::MAX);
                let e = v
                    .get("monthEnd")
                    .and_then(Value::as_i64)
                    .unwrap_or(i64::MIN);
                ordinal >= s && ordinal <= e
            })
        })
        .map(|v| {
            (
                v.get("dawn").and_then(Value::as_f64).unwrap_or(6.0),
                v.get("dusk").and_then(Value::as_f64).unwrap_or(18.0),
            )
        })
        .unwrap_or((6.0, 18.0))
}

fn month_ordinal(cfg: &Value, month: usize) -> i64 {
    cfg.pointer(&format!("/months/values/{month}/ordinal"))
        .and_then(Value::as_i64)
        .unwrap_or(month as i64 + 1)
}

/// Date courante décrite (mois/jour en base 1 pour l'utilisateur).
fn describe_date(cfg: &Value, c: &Comps) -> Value {
    let month_name = cfg
        .pointer(&format!("/months/values/{}/name", c.month))
        .and_then(Value::as_str)
        .map(display_name);
    let weekday = cfg
        .pointer(&format!("/days/values/{}/name", c.day_of_week))
        .and_then(Value::as_str)
        .map(display_name);
    let ordinal = month_ordinal(cfg, c.month);
    let (dawn, dusk) = sun_for(cfg, ordinal);
    json!({
        "date": page_name(c.year, c.month as i64, c.day),
        "year": c.year, "month": c.month + 1, "day": c.day + 1,
        "monthName": month_name, "weekday": weekday,
        "time": format!("{:02}:{:02}:{:02}", c.hour, c.minute, c.second),
        "season": season_for(cfg.pointer("/seasons/values"), ordinal),
        "dawn": dawn, "dusk": dusk,
    })
}

// -------------------------------------------------------------------- dates

/// Nom de la page d'un jour : `YYYY-MM-DD`, mois/jour en base 0 en entrée.
fn page_name(year: i64, month0: i64, day0: i64) -> String {
    format!("{year}-{:02}-{:02}", month0 + 1, day0 + 1)
}

/// `YYYY-MM-DD` (année éventuellement négative) → (année, mois, jour) base 1.
fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let mut parts = body.split('-');
    let y: i64 = parts.next()?.trim().parse().ok()?;
    let m: i64 = parts.next()?.trim().parse().ok()?;
    let d: i64 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((if neg { -y } else { y }, m, d))
}

/// Argument date (chaîne ou objet), mois/jour base 1 validés ≥ 1.
fn parse_date_arg(v: &Value) -> Result<(i64, i64, i64)> {
    let (y, m, d) = match v {
        Value::String(s) => {
            parse_ymd(s).ok_or_else(|| anyhow!("Invalid date \"{s}\": expected YYYY-MM-DD"))?
        }
        Value::Object(o) => {
            let g = |k: &str| {
                o.get(k)
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow!("date.{k} is required (integer)"))
            };
            (g("year")?, g("month")?, g("day")?)
        }
        _ => bail!("date must be \"YYYY-MM-DD\" or {{year, month, day}}"),
    };
    if m < 1 || d < 1 {
        bail!("month and day are 1-based (got month {m}, day {d})");
    }
    Ok((y, m, d))
}

// -------------------------------------------------------------------- notes

/// Rendu HTML d'une page de jour, identique à _saveNotesToSpecificJournal.
fn render_day_html(notes: &[Value]) -> String {
    notes
        .iter()
        .map(|n| {
            format!(
                "<h2><i class=\"{}\"></i> {}</h2><p>{}</p><hr>",
                n.get("icon")
                    .and_then(Value::as_str)
                    .unwrap_or("fas fa-book"),
                n.get("title").and_then(Value::as_str).unwrap_or(""),
                n.get("content").and_then(Value::as_str).unwrap_or(""),
            )
        })
        .collect()
}

/// Rendu HTML de la page des récurrentes (index).
fn render_recurring_html(notes: &[Value]) -> String {
    let mut html = String::from("<h1>Recurring Events Index</h1>");
    for n in notes {
        html.push_str(&format!(
            "<p><strong>{}</strong> ({})</p>",
            n.get("title").and_then(Value::as_str).unwrap_or(""),
            n.get("repeatUnit").and_then(Value::as_str).unwrap_or("")
        ));
    }
    html
}

fn resolve_icon(raw: &str) -> String {
    let r = raw.trim();
    PIN_TYPES
        .iter()
        .find(|(k, l)| k.eq_ignore_ascii_case(r) || l.eq_ignore_ascii_case(r))
        .map(|(k, _)| (*k).to_string())
        .unwrap_or_else(|| {
            if r.contains("fa-") {
                r.to_string()
            } else {
                format!("fas fa-{}", r.to_ascii_lowercase())
            }
        })
}

/// Type de document d'un UUID (« Actor.x », « Compendium.p.q.Item.y »,
/// « Scene.s.Token.t » → le dernier type nommé).
fn uuid_type(uuid: &str) -> Option<String> {
    let parts: Vec<&str> = uuid.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    // Le type est l'avant-dernier segment (l'id est le dernier).
    Some(parts[parts.len() - 2].to_string())
}

fn parse_links(v: &Value) -> Result<Vec<Value>> {
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow!("linked_documents must be an array"))?;
    arr.iter()
        .map(|e| {
            let (uuid, ty) = match e {
                Value::String(s) => (s.clone(), None),
                Value::Object(o) => (
                    o.get("uuid")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    o.get("type").and_then(Value::as_str).map(String::from),
                ),
                _ => bail!("linked_documents entries are UUID strings or {{type, uuid}}"),
            };
            let ty = ty
                .or_else(|| uuid_type(&uuid))
                .ok_or_else(|| anyhow!("Invalid document UUID: \"{uuid}\""))?;
            Ok(json!({"type": ty, "uuid": uuid}))
        })
        .collect()
}

/// Applique les champs modifiables (communs à add et update) sur une note.
fn apply_note_fields(note: &mut Map<String, Value>, args: &Value) -> Result<()> {
    if let Some(t) = str_arg(args, "title") {
        if t.trim().is_empty() {
            bail!("title cannot be empty");
        }
        note.insert("title".into(), json!(t.trim()));
    }
    if let Some(c) = str_arg(args, "content") {
        note.insert("content".into(), json!(c));
    }
    if let Some(i) = str_arg(args, "icon") {
        note.insert("icon".into(), json!(resolve_icon(&i)));
    }
    if args.get("all_day").and_then(Value::as_bool) == Some(true) {
        note.insert("hour".into(), Value::Null);
        note.insert("minute".into(), Value::Null);
    } else {
        if let Some(h) = args.get("hour").and_then(Value::as_i64) {
            note.insert("hour".into(), json!(h));
            if note.get("minute").is_none_or(Value::is_null) {
                note.insert("minute".into(), json!(0));
            }
        }
        if let Some(m) = args.get("minute").and_then(Value::as_i64) {
            note.insert("minute".into(), json!(m));
            if note.get("hour").is_none_or(Value::is_null) {
                note.insert("hour".into(), json!(0));
            }
        }
    }
    if let Some(d) = args.get("duration").and_then(Value::as_i64) {
        note.insert("duration".into(), json!(d.max(1)));
    }
    if let Some(v) = args.get("player_visible").and_then(Value::as_bool) {
        note.insert("playerVisible".into(), json!(v));
    }
    if let Some(l) = args.get("linked_documents") {
        note.insert("linkedDocuments".into(), json!(parse_links(l)?));
    }
    if let Some(m) = args.get("linked_macros").and_then(Value::as_array) {
        note.insert(
            "linkedMacros".into(),
            json!(m.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        );
    }
    if let Some(a) = args.get("auto_execute_macros").and_then(Value::as_bool) {
        note.insert("autoExecuteMacros".into(), json!(a));
    }
    Ok(())
}

/// Nouvelle note ponctuelle, au format de l'éditeur du module.
fn build_note(
    id: &str,
    user_id: &str,
    is_player: bool,
    (y, m, d): (i64, i64, i64),
    args: &Value,
) -> Result<Value> {
    let mut note = Map::new();
    note.insert("id".into(), json!(id));
    note.insert("userId".into(), json!(user_id));
    note.insert("isPlayerNote".into(), json!(is_player));
    note.insert("title".into(), json!(""));
    note.insert("icon".into(), json!("fas fa-book"));
    note.insert("content".into(), json!(""));
    note.insert("hour".into(), Value::Null);
    note.insert("minute".into(), Value::Null);
    note.insert("duration".into(), json!(1));
    note.insert("playerVisible".into(), json!(false));
    note.insert(
        "startDate".into(),
        json!({"year": y, "month": m - 1, "day": d - 1}),
    );
    note.insert("repeatUnit".into(), json!("none"));
    note.insert("repeatInterval".into(), json!(1));
    note.insert("repeatCount".into(), json!(0));
    note.insert("linkedMacros".into(), json!([]));
    note.insert("autoExecuteMacros".into(), json!(false));
    note.insert("linkedDocuments".into(), json!([]));
    apply_note_fields(&mut note, args)?;
    if note["title"].as_str().is_none_or(str::is_empty) {
        bail!("'title' is required");
    }
    Ok(Value::Object(note))
}

fn random_id() -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    (0..16)
        .map(|_| ALPHA[(rand::random::<u32>() as usize) % ALPHA.len()] as char)
        .collect()
}

fn page_notes(page: &Value) -> Vec<Value> {
    page.pointer(&format!("/flags/{MODULE}/notes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn fmt_start_date(sd: Option<&Value>) -> Value {
    match sd {
        Some(s) => match (
            s.get("year").and_then(Value::as_i64),
            s.get("month").and_then(Value::as_i64),
            s.get("day").and_then(Value::as_i64),
        ) {
            (Some(y), Some(m), Some(d)) => json!(page_name(y, m, d)),
            _ => Value::Null,
        },
        None => Value::Null,
    }
}

/// Note telle que présentée par mc_list_notes.
fn present_note(n: &Value, journal: &str, page: &Value, date: Option<&str>) -> Value {
    let mut out = json!({
        "id": n.get("id"), "journal": journal, "pageId": page.get("_id"),
        "title": n.get("title"), "icon": n.get("icon"), "content": n.get("content"),
        "hour": n.get("hour").cloned().unwrap_or(Value::Null),
        "minute": n.get("minute").cloned().unwrap_or(Value::Null),
        "allDay": n.get("hour").is_none_or(Value::is_null),
        "duration": n.get("duration").cloned().unwrap_or(json!(1)),
        "playerVisible": n.get("playerVisible").cloned().unwrap_or(json!(false)),
        "isPlayerNote": n.get("isPlayerNote").cloned().unwrap_or(json!(false)),
        "userId": n.get("userId"),
        "linkedDocuments": n.get("linkedDocuments").cloned().unwrap_or(json!([])),
        "linkedMacros": n.get("linkedMacros").cloned().unwrap_or(json!([])),
        "autoExecuteMacros": n.get("autoExecuteMacros").cloned().unwrap_or(json!(false)),
    });
    if let Some(d) = date {
        out["date"] = json!(d);
    }
    let unit = n
        .get("repeatUnit")
        .and_then(Value::as_str)
        .unwrap_or("none");
    if unit != "none" {
        out["recurrence"] = json!({
            "unit": unit, "interval": n.get("repeatInterval"), "count": n.get("repeatCount"),
            "advancedRule": n.get("advancedRule"), "advParams": n.get("advParams"),
            "startDate": fmt_start_date(n.get("recurrenceStartDate").or_else(|| n.get("startDate"))),
        });
    }
    out
}

/// Journal de notes (doc complet avec pages) ou None.
async fn note_journal(foundry: &FoundryHandle, name: &str) -> Result<Option<Value>> {
    foundry
        .get_document("journal", None, Some(name), None)
        .await
}

fn first_created_id(result: &Value) -> Option<String> {
    result
        .get("result")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|d| d.get("_id").or_else(|| d.as_str().map(|_| d)))
        .and_then(Value::as_str)
        .map(String::from)
}

/// Crée (si besoin) le dossier et le journal de notes ; renvoie l'_id du journal.
async fn ensure_note_journal(foundry: &FoundryHandle, name: &str, player: bool) -> Result<String> {
    if let Some(j) = note_journal(foundry, name).await? {
        return Ok(j["_id"].as_str().unwrap_or_default().to_string());
    }
    let folders = foundry
        .get_documents(
            "folders",
            None,
            Some(&["_id".into(), "name".into(), "type".into()]),
            0,
            None,
        )
        .await?;
    let folder_id = match folders
        .iter()
        .find(|f| f["name"] == json!(NOTES_FOLDER) && f["type"] == json!("JournalEntry"))
    {
        Some(f) => f["_id"].as_str().map(String::from),
        None => {
            let r = foundry
                .modify_document(
                    "Folder",
                    "create",
                    json!({"action": "create", "renderSheet": false, "keepId": false,
                           "data": [{"name": NOTES_FOLDER, "type": "JournalEntry"}]}),
                )
                .await?;
            first_created_id(&r)
        }
    };
    let mut data = json!({"name": name, "folder": folder_id});
    if player {
        data["ownership"] = json!({"default": 3});
    }
    let r = foundry
        .modify_document(
            "JournalEntry",
            "create",
            json!({"action": "create", "renderSheet": false, "keepId": false, "data": [data]}),
        )
        .await?;
    first_created_id(&r).ok_or_else(|| anyhow!("could not create the journal \"{name}\""))
}

/// Écrit la liste de notes d'une page de jour : crée, met à jour ou supprime
/// (liste vide) la page, comme le module.
async fn write_day_page(
    foundry: &FoundryHandle,
    journal_id: &str,
    page: Option<&Value>,
    name: &str,
    notes: &[Value],
) -> Result<Value> {
    let parent = format!("JournalEntry.{journal_id}");
    let recurring = name == RECURRING_PAGE;
    let html = if recurring {
        render_recurring_html(notes)
    } else {
        render_day_html(notes)
    };
    match page {
        Some(p) if notes.is_empty() => {
            foundry
                .modify_document(
                    "JournalEntryPage",
                    "delete",
                    json!({"action": "delete", "parentUuid": parent, "ids": [p["_id"]]}),
                )
                .await
        }
        Some(p) => {
            foundry
                .modify_document(
                    "JournalEntryPage",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "parentUuid": parent,
                        "updates": [{"_id": p["_id"], "text.content": html,
                                     format!("flags.{MODULE}.notes"): notes}],
                    }),
                )
                .await
        }
        None if notes.is_empty() => Ok(Value::Null),
        None => {
            foundry
                .modify_document(
                    "JournalEntryPage",
                    "create",
                    json!({
                        "action": "create", "renderSheet": false, "keepId": false,
                        "parentUuid": parent,
                        "data": [{"name": name, "type": "text",
                                  "text": {"content": html, "format": 1},
                                  "flags": {MODULE: {"notes": notes}}}],
                    }),
                )
                .await
        }
    }
}

/// Où se trouve une note : (clé journal, journal, page, index).
struct Located {
    journal_key: &'static str,
    journal: Value,
    page: Value,
    index: usize,
}

async fn locate_note(foundry: &FoundryHandle, id: &str) -> Result<Located> {
    for (key, jname) in MC_NOTE_JOURNALS {
        let Some(journal) = note_journal(foundry, jname).await? else {
            continue;
        };
        let pages = journal
            .get("pages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for page in pages {
            if let Some(index) = page_notes(&page)
                .iter()
                .position(|n| n.get("id") == Some(&json!(id)))
            {
                return Ok(Located {
                    journal_key: key,
                    journal,
                    page,
                    index,
                });
            }
        }
    }
    bail!("Mini Calendar note not found: {id}")
}

/// Résout un utilisateur (id ou nom) → _id.
async fn resolve_user(foundry: &FoundryHandle, who: &str) -> Result<String> {
    let users = foundry
        .get_documents("users", None, Some(&["_id".into(), "name".into()]), 0, None)
        .await?;
    users
        .iter()
        .find(|u| {
            u["_id"] == json!(who)
                || u["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(who))
        })
        .and_then(|u| u["_id"].as_str().map(String::from))
        .ok_or_else(|| anyhow!("User not found: {who}"))
}

// ------------------------------------------------------------------- météo

fn f_to_c(f: f64) -> i64 {
    ((f - 32.0) * 5.0 / 9.0).floor() as i64
}

fn present_temp(v: Option<&Value>) -> (Value, Value) {
    match v.and_then(Value::as_f64) {
        Some(f) => (json!(f), json!(f_to_c(f))),
        None => (Value::Null, Value::Null),
    }
}

fn present_weather(date: &str, key: &str, entry: Option<&Value>) -> Value {
    let Some(e) = entry else {
        return json!({"date": date, "key": key, "generated": false});
    };
    let (tf, tc) = present_temp(e.get("temp"));
    let mut out = json!({
        "date": date, "key": key, "generated": true,
        "type": e.get("type"), "label": e.get("label"), "icon": e.get("icon"),
        "tempF": tf, "tempC": tc,
        "isManual": e.get("isManual").cloned().unwrap_or(json!(false)),
    });
    if let Some(vars) = e.get("variations").and_then(Value::as_object) {
        let mut v = Map::new();
        for (k, x) in vars {
            let (tf, tc) = present_temp(x.get("temp"));
            v.insert(
                k.clone(),
                json!({"type": x.get("type"), "label": x.get("label"), "tempF": tf, "tempC": tc}),
            );
        }
        out["variations"] = Value::Object(v);
    }
    out
}

// ---------------------------------------------------------------------- run

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        // ---------------------------------------------------- Mini Calendar
        "mc_get_time" => {
            let seconds = world_time(foundry).await?;
            let mut out = json!({"worldTime": seconds as i64});
            if let Ok((source, cfg, assumed)) = load_calendar(foundry).await
                && let Ok(conf) = Conf::from(&cfg)
            {
                let c = conf.time_to_components(seconds.floor() as i64);
                out["calendarSource"] = json!(source);
                out["calendarAssumed"] = json!(assumed);
                out["date"] = describe_date(&cfg, &c);
            }
            Ok(text_response(&out))
        }
        "mc_set_time" => {
            let w = json!({"key": "core.time"}).as_object().cloned().unwrap();
            let docs = foundry
                .get_documents("settings", Some(&w), None, 0, Some(1))
                .await?;
            let doc = docs
                .first()
                .ok_or_else(|| anyhow!("core.time setting not found"))?;
            let current: i64 = doc
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("0")
                .trim()
                .parse()
                .unwrap_or(0);
            let target = match (
                args.get("world_time").and_then(Value::as_i64),
                args.get("advance_seconds").and_then(Value::as_i64),
            ) {
                (Some(t), _) => t,
                (None, Some(d)) => current + d,
                _ => bail!("Provide world_time or advance_seconds"),
            };
            let result = foundry
                .modify_document(
                    "Setting",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{"_id": doc["_id"], "value": target.to_string()}],
                    }),
                )
                .await?;
            Ok(text_response(
                &json!({"worldTime": target, "before": current, "result": result}),
            ))
        }
        "mc_list_notes" => {
            let which = str_arg(args, "which").unwrap_or_else(|| "both".into());
            if !["player", "events", "both"].contains(&which.as_str()) {
                bail!("which must be player, events or both");
            }
            let from = args.get("from").map(parse_date_arg).transpose()?;
            let to = args.get("to").map(parse_date_arg).transpose()?;
            let include_rec = args
                .get("include_recurring")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let mut notes: Vec<(Value, (i64, i64, i64))> = Vec::new();
            let mut recurring = Vec::new();
            let mut journals = Map::new();
            for (key, jname) in MC_NOTE_JOURNALS {
                if which != "both" && which != key {
                    continue;
                }
                let doc = note_journal(foundry, jname).await?;
                journals.insert(
                    key.into(),
                    json!({"name": jname, "found": doc.is_some(), "_id": doc.as_ref().map(|d| d["_id"].clone())}),
                );
                let pages = doc
                    .as_ref()
                    .and_then(|d| d.get("pages"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for page in &pages {
                    let pname = page.get("name").and_then(Value::as_str).unwrap_or_default();
                    if pname == RECURRING_PAGE {
                        if include_rec {
                            recurring.extend(
                                page_notes(page)
                                    .iter()
                                    .map(|n| present_note(n, key, page, None)),
                            );
                        }
                        continue;
                    }
                    let Some(ymd) = parse_ymd(pname) else {
                        continue;
                    };
                    if from.is_some_and(|f| ymd < f) || to.is_some_and(|t| ymd > t) {
                        continue;
                    }
                    for n in page_notes(page) {
                        notes.push((present_note(&n, key, page, Some(pname)), ymd));
                    }
                }
            }
            notes.sort_by(|a, b| {
                a.1.cmp(&b.1).then_with(|| {
                    let h = |v: &Value| {
                        v["hour"].as_i64().unwrap_or(-1) * 60 + v["minute"].as_i64().unwrap_or(0)
                    };
                    h(&a.0).cmp(&h(&b.0))
                })
            });
            let notes: Vec<Value> = notes.into_iter().map(|(n, _)| n).collect();
            let mut out = json!({"journals": journals, "count": notes.len(), "notes": notes});
            if include_rec {
                out["recurring"] = json!(recurring);
            }
            Ok(text_response(&out))
        }
        "mc_add_note" => {
            ensure_module_active(foundry, MODULE, "Mini Calendar").await?;
            let date = parse_date_arg(
                args.get("date")
                    .ok_or_else(|| anyhow!("'date' is required"))?,
            )?;
            let player = match str_arg(args, "journal").as_deref() {
                None | Some("events") => false,
                Some("player") => true,
                Some(o) => bail!("journal must be events or player (got {o})"),
            };
            let user_id = match str_arg(args, "user") {
                Some(u) => resolve_user(foundry, &u).await?,
                None => foundry.user_id().await.unwrap_or_default(),
            };
            let note = build_note(&random_id(), &user_id, player, date, args)?;
            let jname = if player {
                PLAYER_JOURNAL
            } else {
                EVENTS_JOURNAL
            };
            let journal_id = ensure_note_journal(foundry, jname, player).await?;
            let journal = note_journal(foundry, jname).await?.unwrap_or(Value::Null);
            let pname = page_name(date.0, date.1 - 1, date.2 - 1);
            let page = journal
                .get("pages")
                .and_then(Value::as_array)
                .and_then(|ps| ps.iter().find(|p| p["name"] == json!(pname)))
                .cloned();
            let mut notes = page.as_ref().map(page_notes).unwrap_or_default();
            notes.push(note.clone());
            write_day_page(foundry, &journal_id, page.as_ref(), &pname, &notes).await?;
            Ok(text_response(&json!({
                "added": true, "date": pname, "journal": if player { "player" } else { "events" },
                "journalId": journal_id, "note": note,
            })))
        }
        "mc_update_note" => {
            ensure_module_active(foundry, MODULE, "Mini Calendar").await?;
            let id = str_arg(args, "id").ok_or_else(|| anyhow!("'id' is required"))?;
            let loc = locate_note(foundry, &id).await?;
            let pname = loc.page["name"].as_str().unwrap_or_default().to_string();
            if pname == RECURRING_PAGE {
                bail!(
                    "Recurring notes cannot be edited by this tool (only mc_delete_note); edit them in the calendar UI"
                );
            }
            let journal_id = loc.journal["_id"].as_str().unwrap_or_default().to_string();
            let mut notes = page_notes(&loc.page);
            let mut note = notes[loc.index].as_object().cloned().unwrap_or_default();
            apply_note_fields(&mut note, args)?;
            let new_date = args.get("date").map(parse_date_arg).transpose()?;
            let target = new_date.map(|(y, m, d)| page_name(y, m - 1, d - 1));
            match target.filter(|t| *t != pname) {
                None => {
                    notes[loc.index] = Value::Object(note.clone());
                    write_day_page(foundry, &journal_id, Some(&loc.page), &pname, &notes).await?;
                }
                Some(tname) => {
                    let (y, m, d) = new_date.unwrap();
                    note.insert(
                        "startDate".into(),
                        json!({"year": y, "month": m - 1, "day": d - 1}),
                    );
                    notes.remove(loc.index);
                    write_day_page(foundry, &journal_id, Some(&loc.page), &pname, &notes).await?;
                    let dest = loc
                        .journal
                        .get("pages")
                        .and_then(Value::as_array)
                        .and_then(|ps| ps.iter().find(|p| p["name"] == json!(tname)))
                        .cloned();
                    let mut dnotes = dest.as_ref().map(page_notes).unwrap_or_default();
                    dnotes.push(Value::Object(note.clone()));
                    write_day_page(foundry, &journal_id, dest.as_ref(), &tname, &dnotes).await?;
                }
            }
            let date = page_name_of_note(&note).unwrap_or(pname);
            Ok(text_response(&json!({
                "updated": true, "journal": loc.journal_key, "date": date, "note": note,
            })))
        }
        "mc_delete_note" => {
            ensure_module_active(foundry, MODULE, "Mini Calendar").await?;
            let id = str_arg(args, "id").ok_or_else(|| anyhow!("'id' is required"))?;
            let loc = locate_note(foundry, &id).await?;
            let pname = loc.page["name"].as_str().unwrap_or_default().to_string();
            let journal_id = loc.journal["_id"].as_str().unwrap_or_default().to_string();
            let mut notes = page_notes(&loc.page);
            let removed = notes.remove(loc.index);
            write_day_page(foundry, &journal_id, Some(&loc.page), &pname, &notes).await?;
            Ok(text_response(&json!({
                "deleted": true, "journal": loc.journal_key, "page": pname,
                "pageDeleted": notes.is_empty(), "note": {"id": id, "title": removed.get("title")},
            })))
        }
        "mc_get_weather" => {
            let days = args
                .get("days")
                .and_then(Value::as_i64)
                .unwrap_or(6)
                .clamp(1, 60);
            let (source, cfg, assumed) = load_calendar(foundry).await?;
            let conf = Conf::from(&cfg)?;
            let today = conf.time_to_components(world_time(foundry).await?.floor() as i64);
            let start = match args.get("from") {
                Some(v) => {
                    let (y, m, d) = parse_date_arg(v)?;
                    if (m as usize) > conf.months.len() {
                        bail!(
                            "month {m} does not exist in this calendar ({} months)",
                            conf.months.len()
                        );
                    }
                    (y, (m - 1) as usize, d - 1)
                }
                None => (today.year, today.month, today.day),
            };
            let history = read_setting(foundry, &format!("{MODULE}.weatherHistoryStore"))
                .await?
                .unwrap_or(json!({}));
            let use_celsius = read_setting(foundry, &format!("{MODULE}.useCelsius"))
                .await?
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let biome = read_setting(foundry, &format!("{MODULE}.biome"))
                .await?
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "temperate".into());
            let mut cur = start;
            let mut out = Vec::new();
            for _ in 0..days {
                let key = format!("{}-{}-{}", cur.0, cur.1, cur.2);
                let date = page_name(cur.0, cur.1 as i64, cur.2);
                out.push(present_weather(&date, &key, history.get(&key)));
                cur = conf.next_day(cur);
            }
            Ok(text_response(&json!({
                "today": page_name(today.year, today.month as i64, today.day),
                "calendarSource": source, "calendarAssumed": assumed,
                "displayUnit": if use_celsius { "C" } else { "F" }, "biome": biome,
                "days": out,
            })))
        }
        "mc_weather_types" => {
            let custom = read_setting(foundry, &format!("{MODULE}.customWeatherTypes"))
                .await?
                .unwrap_or(json!([]));
            let custom: Vec<Value> = match custom {
                Value::Array(a) => a,
                Value::Object(o) => o.into_iter().map(|(_, v)| v).collect(),
                _ => vec![],
            };
            let builtins: Vec<Value> = BUILTIN_WEATHER
                .iter()
                .map(|(id, label, icon, off)| json!({"id": id, "label": label, "icon": icon, "tempOffsetF": off, "builtin": true}))
                .collect();
            let custom: Vec<Value> = custom
                .iter()
                .filter_map(|e| {
                    let id = e.get("id")?.as_str()?.trim();
                    (!id.is_empty()).then(|| {
                        json!({"id": id, "label": e.get("label").or_else(|| e.get("name")).cloned().unwrap_or(json!(id)),
                               "icon": e.get("icon").cloned().unwrap_or(json!("fas fa-cloud")),
                               "tempOffsetF": e.get("tempOffset"),
                               "fallbackWeatherType": e.get("fallbackWeatherType").or_else(|| e.get("baseWeatherEffect")),
                               "builtin": false})
                    })
                })
                .collect();
            let use_celsius = read_setting(foundry, &format!("{MODULE}.useCelsius"))
                .await?
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let biome = read_setting(foundry, &format!("{MODULE}.biome"))
                .await?
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "temperate".into());
            Ok(text_response(&json!({
                "builtin": builtins, "custom": custom, "biome": biome,
                "displayUnit": if use_celsius { "C" } else { "F" },
                "note": "Biome-specific variants (e.g. \"Drizzle\", \"Sunny\") map to these base ids; temperatures are stored in °F.",
            })))
        }
        "mc_calendar_info" => {
            let (source, cfg, assumed) = load_calendar(foundry).await?;
            let conf = Conf::from(&cfg)?;
            let today = conf.time_to_components(world_time(foundry).await?.floor() as i64);
            let months: Vec<Value> = cfg
                .pointer("/months/values")
                .and_then(Value::as_array)
                .map(|ms| {
                    ms.iter()
                        .enumerate()
                        .map(|(i, m)| {
                            let ordinal = month_ordinal(&cfg, i);
                            let (dawn, dusk) = sun_for(&cfg, ordinal);
                            json!({
                                "index": i + 1, "name": display_name(m.get("name").and_then(Value::as_str).unwrap_or("")),
                                "days": m.get("days"), "leapDays": m.get("leapDays"),
                                "intercalary": m.get("intercalary").cloned().unwrap_or(json!(false)),
                                "season": season_for(cfg.pointer("/seasons/values"), ordinal),
                                "dawn": dawn, "dusk": dusk,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let weekdays: Vec<String> = cfg
                .pointer("/days/values")
                .and_then(Value::as_array)
                .map(|ds| {
                    ds.iter()
                        .map(|d| display_name(d.get("name").and_then(Value::as_str).unwrap_or("")))
                        .collect()
                })
                .unwrap_or_default();
            let names = |p: &str| -> Value {
                cfg.pointer(p)
                    .and_then(Value::as_array)
                    .map(|vs| {
                        json!(vs
                            .iter()
                            .map(|s| json!({"name": display_name(s.get("name").and_then(Value::as_str).unwrap_or("")),
                                            "monthStart": s.get("monthStart"), "monthEnd": s.get("monthEnd")}))
                            .collect::<Vec<_>>())
                    })
                    .unwrap_or(json!([]))
            };
            let moons: Vec<Value> = cfg
                .pointer("/moons/values")
                .and_then(Value::as_array)
                .map(|ms| {
                    ms.iter()
                        .map(|m| json!({
                            "name": m.get("name"), "cycleLength": m.get("cycleLength"),
                            "firstNewMoon": m.get("firstNewMoon"), "color": m.get("color"),
                            "phases": m.get("phases").and_then(Value::as_array).map(|ps| ps.iter()
                                .map(|p| json!({"name": p.get("name"), "length": p.get("length")})).collect::<Vec<_>>()),
                        }))
                        .collect()
                })
                .unwrap_or_default();
            let use_celsius = read_setting(foundry, &format!("{MODULE}.useCelsius"))
                .await?
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let biome = read_setting(foundry, &format!("{MODULE}.biome"))
                .await?
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "temperate".into());
            let leap = if conf.id == "gregorian-preset" {
                json!("gregorian (every 4 years, except centuries not divisible by 400)")
            } else if conf.leap_interval > 0 {
                json!(format!(
                    "every {} years (year divisible by {})",
                    conf.leap_interval, conf.leap_interval
                ))
            } else {
                json!("none")
            };
            let mut out = json!({
                "calendarSource": source, "calendarAssumed": assumed,
                "name": cfg.get("name"), "id": cfg.get("id"),
                "yearZero": conf.year_zero, "leapYears": leap,
                "months": months, "weekdays": weekdays,
                "time": {"hoursPerDay": cfg.pointer("/days/hoursPerDay"), "minutesPerHour": conf.m_per_hour,
                         "secondsPerMinute": conf.s_per_min},
                "seasons": names("/seasons/values"), "weatherSeasons": names("/weather/values"),
                "moons": moons, "biome": biome, "displayUnit": if use_celsius { "C" } else { "F" },
                "today": describe_date(&cfg, &today),
            });
            if assumed {
                out["warning"] = json!(
                    "calendarSource is \"world\": Mini Calendar uses the world/system calendar, which the server cannot read. Showing Foundry's core Simplified Gregorian as an assumption — call client_mc_get_date for the live calendar."
                );
            }
            Ok(text_response(&out))
        }
        "client_mc_set_time" => {
            let mut a = json!({});
            carry(
                args,
                &[
                    "world_time",
                    "advance_seconds",
                    "day",
                    "hour",
                    "minute",
                    "next",
                ],
                &mut a,
            );
            if let Some(m) = str_arg(args, "mode") {
                a["mode"] = json!(m);
            }
            let r = call_companion(state, cmd_for(name), a, None, 10).await?;
            Ok(text_response(&r))
        }
        "client_mc_open" => {
            let mut a = json!({});
            carry(args, &["toggle"], &mut a);
            let r = call_companion(state, cmd_for(name), a, None, 10).await?;
            Ok(text_response(&r))
        }
        "client_mc_get_date" => {
            let r = call_companion(state, cmd_for(name), json!({}), None, 10).await?;
            Ok(text_response(&r))
        }
        "client_mc_set_date" => {
            for k in ["year", "month", "day"] {
                if args.get(k).and_then(Value::as_i64).is_none() {
                    bail!("'{k}' is required (integer; month and day are 1-based)");
                }
            }
            let mut a = json!({});
            carry(args, &["year", "month", "day", "hour", "minute"], &mut a);
            let r = call_companion(state, cmd_for(name), a, None, 10).await?;
            Ok(text_response(&r))
        }
        "client_mc_set_weather" => {
            let ty = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            let unit = str_arg(args, "unit")
                .unwrap_or_else(|| "C".into())
                .to_ascii_uppercase();
            let mut a = json!({"type": ty, "day_delta": args.get("day_delta").and_then(Value::as_i64).unwrap_or(0).max(0)});
            if let Some(t) = args.get("temp").and_then(Value::as_f64) {
                a["temp_f"] = json!(match unit.as_str() {
                    "F" => t.round() as i64,
                    "C" => (t * 9.0 / 5.0 + 32.0).round() as i64,
                    o => bail!("unit must be C or F (got {o})"),
                });
            }
            let r = call_companion(state, cmd_for(name), a, None, 15).await?;
            Ok(text_response(&r))
        }

        other => bail!("Unknown tool: {other}"),
    }
}

fn page_name_of_note(note: &Map<String, Value>) -> Option<String> {
    fmt_start_date(note.get("startDate"))
        .as_str()
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn greg() -> Conf {
        Conf::from(&DEFAULT_CALENDAR).unwrap()
    }

    #[test]
    fn noms_de_page() {
        assert_eq!(page_name(1492, 0, 6), "1492-01-07");
        assert_eq!(page_name(3, 11, 30), "3-12-31");
        assert_eq!(parse_ymd("1492-03-07"), Some((1492, 3, 7)));
        assert_eq!(parse_ymd("-20-01-02"), Some((-20, 1, 2)));
        assert_eq!(parse_ymd("0000-Recurring"), None);
        assert_eq!(parse_ymd("1-2"), None);
    }

    #[test]
    fn argument_date() {
        assert_eq!(parse_date_arg(&json!("12-3-4")).unwrap(), (12, 3, 4));
        assert_eq!(
            parse_date_arg(&json!({"year": 5, "month": 1, "day": 9})).unwrap(),
            (5, 1, 9)
        );
        assert!(parse_date_arg(&json!("12-0-4")).is_err());
        assert!(parse_date_arg(&json!({"year": 5})).is_err());
        assert!(parse_date_arg(&json!(3)).is_err());
    }

    #[test]
    fn gregorien_comme_le_module() {
        let c = greg();
        // Monde du banc : 21600 s = an 0, 1er janvier, 6 h (année 0 bissextile
        // pour la règle « year % leapInterval » du module).
        let x = c.time_to_components(21_600);
        assert_eq!((x.year, x.month, x.day, x.hour), (0, 0, 0, 6));
        assert!(c.is_leap(0));
        // 366 jours plus tard (an 0 bissextile) → 1er janvier an 1.
        let x = c.time_to_components(366 * 86_400);
        assert_eq!((x.year, x.month, x.day), (1, 0, 0));
        // 29 février de l'an 0.
        let x = c.time_to_components((31 + 28) * 86_400 + 3_600 * 13 + 125);
        assert_eq!(
            (x.month, x.day, x.hour, x.minute, x.second),
            (1, 28, 13, 2, 5)
        );
        // Lendemain : bascule de mois et d'année.
        assert_eq!(c.next_day((0, 1, 28)), (0, 2, 0));
        assert_eq!(c.next_day((1, 1, 27)), (1, 2, 0));
        assert_eq!(c.next_day((1, 11, 30)), (2, 0, 0));
    }

    #[test]
    fn calendrier_sans_bissextile_et_intercalaire() {
        let cfg = json!({
            "years": {"yearZero": 1000, "firstWeekday": 2},
            "months": {"values": [{"days": 10}, {"days": 1, "intercalary": true}, {"days": 10}]},
            "days": {"values": [{"name":"A"},{"name":"B"},{"name":"C"}], "hoursPerDay": 10, "minutesPerHour": 10, "secondsPerMinute": 10},
        });
        let c = Conf::from(&cfg).unwrap();
        // 1 jour = 1000 s, 1 an = 21 jours.
        let x = c.time_to_components(21_000 + 10_500);
        assert_eq!((x.year, x.month, x.day, x.hour), (1001, 1, 0, 5));
        // Jour de semaine : 31 jours écoulés − 1 intercalaire (an 1000) + 2.
        assert_eq!(x.day_of_week, (31 - 1 + 2) % 3);
    }

    #[test]
    fn rendu_html_identique_au_module() {
        let notes = vec![json!({"icon": "fas fa-scroll", "title": "T", "content": "C"})];
        assert_eq!(
            render_day_html(&notes),
            "<h2><i class=\"fas fa-scroll\"></i> T</h2><p>C</p><hr>"
        );
        let rec = vec![json!({"title": "Fête", "repeatUnit": "years"})];
        assert_eq!(
            render_recurring_html(&rec),
            "<h1>Recurring Events Index</h1><p><strong>Fête</strong> (years)</p>"
        );
    }

    #[test]
    fn construction_de_note() {
        let n = build_note("abc", "u1", false, (1492, 3, 7), &json!({
            "title": " Marché ", "icon": "quest", "hour": 14, "duration": 0,
            "player_visible": true, "linked_documents": ["Actor.x1", {"type": "Item", "uuid": "Compendium.p.q.Item.y"}],
        })).unwrap();
        assert_eq!(n["title"], "Marché");
        assert_eq!(n["icon"], "fas fa-scroll");
        assert_eq!(
            (n["hour"].as_i64(), n["minute"].as_i64()),
            (Some(14), Some(0))
        );
        assert_eq!(n["duration"], 1);
        assert_eq!(n["startDate"], json!({"year": 1492, "month": 2, "day": 6}));
        assert_eq!(n["repeatUnit"], "none");
        assert_eq!(
            n["linkedDocuments"][0],
            json!({"type": "Actor", "uuid": "Actor.x1"})
        );
        assert_eq!(n["linkedDocuments"][1]["type"], "Item");
        assert!(build_note("a", "u", false, (1, 1, 1), &json!({})).is_err());
        let mut m = n.as_object().cloned().unwrap();
        apply_note_fields(&mut m, &json!({"all_day": true, "icon": "fas fa-gem"})).unwrap();
        assert!(m["hour"].is_null() && m["minute"].is_null());
        assert_eq!(m["icon"], "fas fa-gem");
    }

    #[test]
    fn uuid_et_icones() {
        assert_eq!(
            uuid_type("JournalEntry.abc").as_deref(),
            Some("JournalEntry")
        );
        assert_eq!(
            uuid_type("Compendium.world.pack.Actor.id").as_deref(),
            Some("Actor")
        );
        assert_eq!(uuid_type("abc"), None);
        assert_eq!(resolve_icon("Moon/Eclipse"), "fas fa-moon");
        assert_eq!(resolve_icon("dragon"), "fas fa-dragon");
        assert_eq!(resolve_icon("fa-solid fa-cat"), "fa-solid fa-cat");
    }

    #[test]
    fn meteo_et_saisons() {
        assert_eq!(f_to_c(50.0), 10);
        assert_eq!(f_to_c(33.0), 0);
        assert_eq!(f_to_c(31.0), -1);
        let w = present_weather(
            "0-01-01",
            "0-0-0",
            Some(
                &json!({"type": "rain", "temp": 50, "variations": {"morning": {"type": "fog", "temp": 41}}}),
            ),
        );
        assert_eq!(w["tempC"], 10);
        assert_eq!(w["variations"]["morning"]["tempC"], 5);
        assert_eq!(present_weather("d", "k", None)["generated"], false);
        let seasons = DEFAULT_CALENDAR.pointer("/seasons/values");
        assert_eq!(season_for(seasons, 1).as_deref(), Some("Winter"));
        assert_eq!(season_for(seasons, 7).as_deref(), Some("Summer"));
        assert_eq!(display_name("CALENDAR.GREGORIAN.January"), "January");
        assert_eq!(display_name("Mois des Brumes"), "Mois des Brumes");
        assert_eq!(sun_for(&DEFAULT_CALENDAR, 3), (6.0, 18.0));
    }
}
