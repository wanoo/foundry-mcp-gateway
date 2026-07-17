//! Filtres et projections de documents — port fidèle de la sémantique validée :
//! `where` à chemins pointés + opérateurs __in / __contains / __ne / __exists,
//! pushdown des égalités simples vers la query serveur, index BDD pour les
//! listings _id/name.

use serde_json::{Map, Value};

pub fn get_path<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = doc;
    for seg in path.split('.') {
        cur = cur.as_object()?.get(seg)?;
    }
    Some(cur)
}

fn op_of(key: &str) -> (&str, &str) {
    for op in ["__in", "__contains", "__ne", "__exists"] {
        if let Some(path) = key.strip_suffix(op) {
            return (path, &op[2..]);
        }
    }
    (key, "eq")
}

pub fn matches_where(doc: &Value, where_: &Map<String, Value>) -> bool {
    for (key, expected) in where_ {
        let (path, op) = op_of(key);
        let actual = get_path(doc, path);
        let ok = match op {
            "eq" => actual == Some(expected) || (actual.is_none() && expected.is_null()),
            "ne" => !(actual == Some(expected) || (actual.is_none() && expected.is_null())),
            "in" => match (expected.as_array(), actual) {
                (Some(list), Some(a)) => list.contains(a),
                _ => false,
            },
            "contains" => match (actual, expected) {
                (Some(Value::String(s)), Value::String(needle)) => {
                    s.to_lowercase().contains(&needle.to_lowercase())
                }
                (Some(Value::Array(arr)), v) => arr.contains(v),
                _ => false,
            },
            "exists" => {
                let wants = expected.as_bool().unwrap_or(false);
                actual.is_some() == wants
            }
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Pushdown serveur : égalités top-level (sans opérateur ni chemin) + _id__in.
pub fn pushdown_query(where_: Option<&Map<String, Value>>) -> Map<String, Value> {
    let mut query = Map::new();
    if let Some(w) = where_ {
        for (key, value) in w {
            if key == "_id__in" && value.is_array() {
                query.insert(key.clone(), value.clone());
            } else {
                let (path, op) = op_of(key);
                if op == "eq" && !path.contains('.') {
                    query.insert(key.clone(), value.clone());
                }
            }
        }
    }
    query
}

/// L'index BDD n'est sûr que pour des listings/filtres limités à _id/name.
pub fn can_use_index(fields: Option<&[String]>, where_: Option<&Map<String, Value>>) -> bool {
    let safe = |s: &str| s == "_id" || s == "name";
    let Some(fields) = fields else { return false };
    if fields.is_empty() || !fields.iter().all(|f| safe(f)) {
        return false;
    }
    if let Some(w) = where_ {
        for key in w.keys() {
            let (path, _) = op_of(key);
            if !safe(path) {
                return false;
            }
        }
    }
    true
}

/// Projection : garde les champs demandés (toujours _id et name).
pub fn filter_fields(doc: &Value, fields: Option<&[String]>) -> Value {
    let Some(fields) = fields else { return doc.clone() };
    if fields.is_empty() {
        return doc.clone();
    }
    let Some(obj) = doc.as_object() else { return doc.clone() };
    let mut out = Map::new();
    for key in fields.iter().map(String::as_str).chain(["_id", "name"]) {
        if let Some(v) = obj.get(key) {
            out.entry(key.to_string()).or_insert_with(|| v.clone());
        }
    }
    Value::Object(out)
}

/// Les 13 collections monde → type de document Foundry.
pub fn collection_to_type(collection: &str) -> Option<&'static str> {
    Some(match collection {
        "actors" => "Actor",
        "items" => "Item",
        "folders" => "Folder",
        "users" => "User",
        "scenes" => "Scene",
        "journal" => "JournalEntry",
        "macros" => "Macro",
        "cards" => "Cards",
        "playlists" => "Playlist",
        "tables" => "RollTable",
        "combats" => "Combat",
        "messages" => "ChatMessage",
        "settings" => "Setting",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn docs() -> Vec<Value> {
        vec![
            json!({"_id":"a","name":"Riar Starport","flags":{"campaign-codex":{"type":"location"}},"folder":"f1"}),
            json!({"_id":"b","name":"Jerserra","flags":{"campaign-codex":{"type":"npc"}},"folder":null}),
            json!({"_id":"c","name":"Halyard","flags":{},"folder":"f2"}),
        ]
    }

    #[test]
    fn where_chemins_et_operateurs() {
        let d = docs();
        let w = |v: Value| v.as_object().unwrap().clone();
        let keep = |w_: &Map<String, Value>| {
            d.iter().filter(|x| matches_where(x, w_)).map(|x| x["_id"].clone()).collect::<Vec<_>>()
        };
        assert_eq!(keep(&w(json!({"flags.campaign-codex.type":"npc"}))), vec![json!("b")]);
        assert_eq!(keep(&w(json!({"_id__in":["a","c"]}))), vec![json!("a"), json!("c")]);
        assert_eq!(keep(&w(json!({"name__contains":"RIAR"}))), vec![json!("a")]);
        assert_eq!(keep(&w(json!({"folder__ne":null}))), vec![json!("a"), json!("c")]);
        assert_eq!(keep(&w(json!({"flags.campaign-codex__exists":true}))), vec![json!("a"), json!("b")]);
        assert_eq!(keep(&w(json!({"flags.nope.deep":"x"}))), Vec::<Value>::new());
    }

    #[test]
    fn pushdown_et_index() {
        let w = json!({"type":"npc","name__contains":"a","flags.x.y":"z","_id__in":["a"]});
        let q = pushdown_query(Some(w.as_object().unwrap()));
        assert_eq!(q.len(), 2);
        assert!(q.contains_key("type") && q.contains_key("_id__in"));

        let f = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(can_use_index(Some(&f(&["_id", "name"])), None));
        assert!(!can_use_index(None, None));
        assert!(!can_use_index(Some(&f(&["_id", "type"])), None));
        let w2 = json!({"folder":"f1"});
        assert!(!can_use_index(Some(&f(&["_id"])), Some(w2.as_object().unwrap())));
    }

    #[test]
    fn projection() {
        let d = json!({"_id":"a","name":"X","type":"npc","big":{"x":1}});
        let out = filter_fields(&d, Some(&["type".to_string()]));
        assert_eq!(out, json!({"_id":"a","name":"X","type":"npc"}));
    }
}
