//! Plan d'administration (/setup) — protocole vérifié dans le code client v13
//! servi par l'instance (foundry.mjs) : tout est POST JSON `{action, …}`.
//!   · editWorld  → POST /setup, accepté PENDANT que le monde tourne (session
//!     de jeu du bot — c'est ce que fait l'écran « Modifier le monde » du MJ).
//!   · shutdown   → POST /join {action:"shutdown", adminPassword}.
//!   · launchWorld / checkPackage / installPackage → POST /setup, monde ÉTEINT,
//!     après authentification admin sur /auth (formulaire HTML : on tente JSON
//!     puis formulaire, et on VÉRIFIE en re-GET /setup).
//! La progression (launch/install) est diffusée en events `progress` — visibles
//! via get_events une fois le monde revenu.
//!
//! Outils exposés seulement si FOUNDRY_ADMIN_PASSWORD est présent (sauf
//! admin_edit_world qui n'en a pas besoin : session de jeu suffit).

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use super::{str_arg, text_response};
use crate::foundry::auth::split_host;
use crate::mcp::McpState;

pub fn definitions(state: &McpState) -> Vec<(&'static str, &'static str, Value)> {
    let mut tools = vec![
        ("admin_status",
         "Instance status straight from /api/status: is a world active, which one, version, uptime — works even when the world is down (unlike every world tool). Also says whether the admin password is configured for the admin_* tools.",
         json!({"type":"object","properties":{}})),
        ("admin_edit_world",
         "Edit the world's metadata: title, description (HTML), background image (path or URL), next session date (ISO). Works while the world is running; players see it on the join page. Only the given fields change.",
         json!({"type":"object","properties":{
            "title":{"type":"string"},
            "description":{"type":"string"},
            "background":{"type":"string","description":"image path/URL shown on the join page"},
            "next_session":{"type":"string","description":"ISO date, or empty string to clear"}}})),
        ("manage_modules",
         "List the world's modules (installed vs enabled, with versions), or enable/disable some. Changes need every client to reload (F5) to take effect — the tool reminds you.",
         json!({"type":"object","properties":{
            "enable":{"type":"array","items":{"type":"string"},"description":"module ids to enable"},
            "disable":{"type":"array","items":{"type":"string"},"description":"module ids to disable"}}})),
        ("manage_users",
         "List the world's user accounts, or create/update/delete them: name, role (player/trusted/assistant/gamemaster), assigned character, colour. PASSWORDS ARE NOT HANDLED HERE — the GM sets them in Foundry (Configure Players); accounts created here start password-less, which Foundry allows.",
         json!({"type":"object","properties":{
            "create":{"type":"array","description":"[{name, role?, character?, color?}] — role: player|trusted|assistant|gamemaster (default player)",
                      "items":{"type":"object"}},
            "update":{"type":"array","description":"[{_id or name, role?, character?, color?}]",
                      "items":{"type":"object"}},
            "delete":{"type":"array","items":{"type":"string"},"description":"user _ids or names to delete (refuses the bot's own account)"}}})),
    ];
    if state.admin_password.is_some() {
        tools.extend([
            ("admin_shutdown_world",
             "Shut the running world down (back to setup mode). DISCONNECTS every player AND this MCP server's bot (world tools go down until relaunch; the bot reconnects automatically). Requires confirm:true.",
             json!({"type":"object","properties":{
                "confirm":{"type":"boolean","description":"must be true"}},
                "required":["confirm"]})),
            ("admin_launch_world",
             "Launch a world from setup mode (after admin_shutdown_world). Default: the last world seen active. The bot reconnects automatically once it is up (~10-30 s).",
             json!({"type":"object","properties":{
                "world":{"type":"string","description":"world id, e.g. star-wars"}}})),
            ("admin_list_backups",
             "List the backups Foundry holds (worlds, systems, modules) with their date, size and note. Setup mode only.",
             json!({"type":"object","properties":{}})),
            ("admin_backup_world",
             "Create a backup before doing something risky. Setup mode only (shut the world down first). Defaults to the last active world; pass type/id for a module or system instead.",
             json!({"type":"object","properties":{
                "type":{"type":"string","description":"world (default) | module | system"},
                "id":{"type":"string","description":"package id; defaults to the last active world"},
                "note":{"type":"string","description":"why you took it — shown in Foundry's backup list"}}})),
            ("admin_check_package",
             "Check if an update is available for a module, system or world (compares installed vs remote manifest). Setup mode only (shut the world down first).",
             json!({"type":"object","properties":{
                "type":{"type":"string","description":"module | system | world (default module)"},
                "id":{"type":"string"}},
                "required":["id"]})),
            ("admin_update_package",
             "Update a module, system or world to its latest version. Setup mode only — refuses if a world is running. Takes a BACKUP first (unless backup:false), checks, installs only if an update exists, retries up to 5 times, then verifies.",
             json!({"type":"object","properties":{
                "type":{"type":"string","description":"module | system | world (default module)"},
                "id":{"type":"string"},
                "backup":{"type":"boolean","description":"back the package up before updating (default true)"}},
                "required":["id"]})),
        ]);
    }
    tools
}

pub fn handles(name: &str) -> bool {
    name.starts_with("admin_") || name == "manage_modules"
}

/* ------------------------------------------------------------- plumbing */

fn base_url(state: &McpState) -> String {
    let (host, base) = split_host(&state.foundry.hostname());
    format!("https://{host}{base}")
}

fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .context("client HTTP")
}

async fn api_status(url: &str) -> Result<Value> {
    Ok(reqwest::get(format!("{url}/api/status"))
        .await?
        .json()
        .await?)
}

/// Authentifie la session du client `http` sur /auth (mode setup uniquement).
/// Tente JSON puis formulaire, et VÉRIFIE : /setup ne doit plus rediriger
/// vers /auth.
async fn admin_auth(http: &reqwest::Client, url: &str, password: &str) -> Result<()> {
    let is_authed = |resp: &reqwest::Response| {
        !resp.url().path().ends_with("/auth") && !resp.url().path().ends_with("/join")
    };
    // amorce la session
    let _ = http.get(format!("{url}/auth")).send().await;
    for attempt in 0..2 {
        let req = http.post(format!("{url}/auth"));
        let req = if attempt == 0 {
            req.json(&json!({ "action": "adminAuth", "adminPassword": password }))
        } else {
            req.form(&[("action", "adminAuth"), ("adminPassword", password)])
        };
        let _ = req.send().await;
        let check = http.get(format!("{url}/setup")).send().await?;
        if is_authed(&check) {
            return Ok(());
        }
    }
    bail!("authentification admin refusée (mot de passe FOUNDRY_ADMIN_PASSWORD incorrect ?)")
}

async fn setup_post(http: &reqwest::Client, url: &str, body: &Value) -> Result<Value> {
    let resp = http
        .post(format!("{url}/setup"))
        .json(body)
        .send()
        .await
        .context("POST /setup")?;
    let status = resp.status();
    let data: Value = resp.json().await.unwrap_or(Value::Null);
    if let Some(err) = data.get("error").and_then(Value::as_str) {
        bail!("setup a répondu une erreur : {err}");
    }
    if !status.is_success() {
        bail!("POST /setup {} → HTTP {status} : {data}", body["action"]);
    }
    Ok(data)
}

/// createBackup : l'id porte la convention de Foundry (`type.pkg.AAAA-MM-JJ.epoch`)
/// — un timestamp est nécessaire, on le prend sur l'horloge système.
async fn run_backup(
    http: &reqwest::Client,
    url: &str,
    pkg_type: &str,
    id: &str,
    note: &str,
) -> Result<Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let days = now / 86_400_000;
    // date civile depuis l'epoch (algorithme de Howard Hinnant, sans dépendance)
    let (y, m, d) = {
        let z = days as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (era * 400 + yoe + i64::from(m <= 2), m, d)
    };
    let backup_id = format!("{pkg_type}.{id}.{y:04}-{m:02}-{d:02}.{now}");
    let body = json!({
        "action": "createBackup",
        "backups": [{ "type": pkg_type, "packageId": id, "note": note, "id": backup_id }],
    });
    let data = setup_post(http, url, &body).await?;
    Ok(json!({
        "backedUp": id, "type": pkg_type, "backupId": backup_id,
        "note": note, "response": data,
    }))
}

fn admin_password(state: &McpState) -> Result<&str> {
    state
        .admin_password
        .as_deref()
        .map(String::as_str)
        .ok_or_else(|| anyhow!("FOUNDRY_ADMIN_PASSWORD n'est pas configuré sur le serveur"))
}

/* ---------------------------------------------------------------- tools */

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let url = base_url(state);
    match name {
        "admin_status" => {
            let status = api_status(&url).await?;
            if let Some(w) = status.get("world").and_then(Value::as_str) {
                *state.last_world_id.lock().await = Some(w.to_string());
            }
            Ok(text_response(&json!({
                "status": status,
                "adminPasswordConfigured": state.admin_password.is_some(),
            })))
        }

        "admin_edit_world" => {
            let status = api_status(&url).await?;
            let world_id = status.get("world").and_then(Value::as_str).ok_or_else(|| {
                anyhow!("aucun monde actif — admin_edit_world passe par la session de jeu du bot")
            })?;
            // état courant (le formulaire envoie le document complet) —
            // les métadonnées vivent sous la clé `world` du dump.
            let dump = state.foundry.request_world().await?;
            let world = dump.get("world").cloned().unwrap_or(Value::Null);
            let current = |k: &str| world.get(k).cloned().unwrap_or(Value::Null);
            let mut payload = Map::new();
            payload.insert("action".into(), json!("editWorld"));
            payload.insert("id".into(), json!(world_id));
            for (arg, field) in [
                ("title", "title"),
                ("description", "description"),
                ("background", "background"),
                ("next_session", "nextSession"),
            ] {
                let v = match str_arg(args, arg) {
                    Some(s) if s.is_empty() => Value::Null,
                    Some(s) => json!(s),
                    None => current(field),
                };
                payload.insert(field.into(), v);
            }
            // session de jeu du bot (même canal que l'écran MJ « Edit World »)
            let session = state
                .foundry
                .session_id()
                .await
                .ok_or_else(|| anyhow!("session Foundry indisponible (bot déconnecté ?)"))?;
            let resp = reqwest::Client::new()
                .post(format!("{url}/setup"))
                .header(reqwest::header::COOKIE, format!("session={session}"))
                .json(&Value::Object(payload.clone()))
                .send()
                .await
                .context("POST /setup editWorld")?;
            let status_code = resp.status();
            let data: Value = resp.json().await.unwrap_or(Value::Null);
            if let Some(err) = data.get("error").and_then(Value::as_str) {
                bail!("editWorld refusé : {err}");
            }
            if !status_code.is_success() {
                bail!("editWorld → HTTP {status_code} : {data}");
            }
            Ok(text_response(&json!({
                "edited": world_id,
                "title": payload["title"], "background": payload["background"],
                "nextSession": payload["nextSession"],
                "note": "visible sur la page de connexion (les joueurs en jeu ne voient rien changer)",
            })))
        }

        "manage_modules" => {
            // état : liste installée (dump monde) × configuration activée (setting)
            let world = state.foundry.request_world().await?;
            let empty = vec![];
            let installed = world
                .get("modules")
                .and_then(Value::as_array)
                .unwrap_or(&empty);
            let w = json!({"key": "core.moduleConfiguration"})
                .as_object()
                .cloned()
                .unwrap();
            let setting = state
                .foundry
                .get_documents("settings", Some(&w), None, 0, Some(1))
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("core.moduleConfiguration introuvable"))?;
            let mut config: Map<String, Value> = setting
                .get("value")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();

            let ids = |key: &str| -> Vec<String> {
                args.get(key)
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let (to_enable, to_disable) = (ids("enable"), ids("disable"));
            if to_enable.is_empty() && to_disable.is_empty() {
                // lecture seule : le tableau de bord
                let list: Vec<Value> = installed
                    .iter()
                    .map(|m| {
                        let id = m.get("id").and_then(Value::as_str).unwrap_or("?");
                        json!({
                            "id": id,
                            "title": m.get("title"),
                            "version": m.get("version"),
                            "enabled": config.get(id).and_then(Value::as_bool).unwrap_or(false),
                        })
                    })
                    .collect();
                let enabled = list.iter().filter(|m| m["enabled"] == json!(true)).count();
                return Ok(text_response(&json!({
                    "installed": list.len(), "enabled": enabled, "modules": list,
                })));
            }
            // écriture : vérifier que les ids existent avant de toucher au réglage
            let known: Vec<&str> = installed
                .iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str))
                .collect();
            for id in to_enable.iter().chain(to_disable.iter()) {
                if !known.contains(&id.as_str()) {
                    bail!("module inconnu : {id} (voir manage_modules sans argument)");
                }
            }
            for id in &to_enable {
                config.insert(id.clone(), json!(true));
            }
            for id in &to_disable {
                config.insert(id.clone(), json!(false));
            }
            state
                .foundry
                .modify_document(
                    "Setting",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{
                            "_id": setting["_id"],
                            "value": Value::Object(config).to_string(),
                        }],
                    }),
                )
                .await?;
            Ok(text_response(&json!({
                "enabled": to_enable, "disabled": to_disable,
                "note": "chaque client doit recharger (F5) pour que le changement prenne effet",
            })))
        }

        "manage_users" => {
            let bot = state.foundry.user_id().await;
            let users = state
                .foundry
                .get_documents("users", None, None, 0, None)
                .await?;
            let find = |key: &str| -> Option<Value> {
                users
                    .iter()
                    .find(|u| u["_id"] == json!(key) || u["name"] == json!(key))
                    .cloned()
            };
            let role_of = |v: Option<&Value>| -> Result<Option<i64>> {
                let Some(v) = v else { return Ok(None) };
                if let Some(n) = v.as_i64() {
                    return Ok(Some(n));
                }
                let s = v.as_str().unwrap_or_default().to_lowercase();
                Ok(Some(match s.as_str() {
                    "none" => 0,
                    "player" => 1,
                    "trusted" => 2,
                    "assistant" | "assistant gm" | "assistantgm" => 3,
                    "gamemaster" | "gm" => 4,
                    other => bail!(
                        "rôle inconnu : '{other}' (player | trusted | assistant | gamemaster)"
                    ),
                }))
            };
            let label = |r: i64| match r {
                0 => "none",
                1 => "player",
                2 => "trusted",
                3 => "assistant",
                4 => "gamemaster",
                _ => "?",
            };
            let (create, update, delete) = (
                args.get("create").and_then(Value::as_array),
                args.get("update").and_then(Value::as_array),
                args.get("delete").and_then(Value::as_array),
            );
            // lecture seule : le tableau de bord des comptes
            if create.is_none() && update.is_none() && delete.is_none() {
                let list: Vec<Value> = users
                    .iter()
                    .map(|u| {
                        let r = u.get("role").and_then(Value::as_i64).unwrap_or(0);
                        json!({
                            "_id": u["_id"], "name": u["name"],
                            "role": r, "roleName": label(r),
                            "character": u.get("character"),
                            "hasPassword": u.get("password").and_then(Value::as_str)
                                .map(|p| !p.is_empty()).unwrap_or(false),
                        })
                    })
                    .collect();
                return Ok(text_response(
                    &json!({ "count": list.len(), "users": list }),
                ));
            }
            let mut report = json!({});
            if let Some(items) = create {
                let mut data = Vec::new();
                for item in items {
                    let name = str_arg(item, "name")
                        .ok_or_else(|| anyhow!("chaque compte à créer a besoin d'un 'name'"))?;
                    if find(&name).is_some() {
                        bail!("un utilisateur nommé '{name}' existe déjà");
                    }
                    let mut doc = json!({
                        "name": name,
                        "role": role_of(item.get("role"))?.unwrap_or(1),
                    });
                    for k in ["character", "color"] {
                        if let Some(v) = item.get(k) {
                            doc[k] = v.clone();
                        }
                    }
                    data.push(doc);
                }
                let r = state
                    .foundry
                    .modify_document(
                        "User",
                        "create",
                        json!({ "action": "create", "broadcast": false, "renderSheet": false,
                                "keepId": false, "data": data }),
                    )
                    .await?;
                report["created"] = r
                    .get("result")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|u| json!({"_id": u["_id"], "name": u["name"], "role": u["role"]}))
                            .collect::<Vec<_>>()
                            .into()
                    })
                    .unwrap_or(Value::Null);
                report["passwordNote"] = json!(
                    "comptes sans mot de passe — à définir dans Foundry (Configurer les joueurs)"
                );
            }
            if let Some(items) = update {
                let mut updates = Vec::new();
                for item in items {
                    let key = str_arg(item, "_id")
                        .or_else(|| str_arg(item, "name"))
                        .ok_or_else(|| anyhow!("chaque mise à jour a besoin de '_id' ou 'name'"))?;
                    let existing =
                        find(&key).ok_or_else(|| anyhow!("utilisateur introuvable : {key}"))?;
                    let mut u = json!({ "_id": existing["_id"] });
                    if let Some(r) = role_of(item.get("role"))? {
                        u["role"] = json!(r);
                    }
                    for k in ["character", "color", "name"] {
                        if let Some(v) = item.get(k) {
                            u[k] = v.clone();
                        }
                    }
                    updates.push(u);
                }
                state
                    .foundry
                    .modify_document(
                        "User",
                        "update",
                        json!({ "action": "update", "diff": false, "recursive": true,
                                "render": true, "updates": updates }),
                    )
                    .await?;
                report["updated"] = json!(updates.len());
            }
            if let Some(items) = delete {
                let mut ids = Vec::new();
                for item in items {
                    let key = item.as_str().unwrap_or_default();
                    let existing =
                        find(key).ok_or_else(|| anyhow!("utilisateur introuvable : {key}"))?;
                    let id = existing["_id"].as_str().unwrap_or_default().to_string();
                    if Some(&id) == bot.as_ref() {
                        bail!("refus de supprimer le compte du bot MCP lui-même ({key})");
                    }
                    ids.push(json!(id));
                }
                state
                    .foundry
                    .modify_document(
                        "User",
                        "delete",
                        json!({ "action": "delete", "broadcast": false, "deleteAll": false,
                                "ids": ids }),
                    )
                    .await?;
                report["deleted"] = json!(ids);
            }
            Ok(text_response(&report))
        }

        "admin_list_backups" => {
            let password = admin_password(state)?;
            let status = api_status(&url).await?;
            if let Some(active) = status.get("world").and_then(Value::as_str) {
                bail!("le monde '{active}' tourne — les sauvegardes exigent le mode setup");
            }
            let client = http()?;
            admin_auth(&client, &url, password).await?;
            let data = setup_post(&client, &url, &json!({ "action": "listBackups" })).await?;
            Ok(text_response(&data))
        }

        "admin_backup_world" => {
            let password = admin_password(state)?;
            let pkg_type = str_arg(args, "type").unwrap_or_else(|| "world".into());
            let id = match str_arg(args, "id") {
                Some(i) => i,
                None => state
                    .last_world_id
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| anyhow!("précisez 'id' (aucun monde vu actif récemment)"))?,
            };
            let note = str_arg(args, "note").unwrap_or_else(|| "sauvegarde via MCP".into());
            let status = api_status(&url).await?;
            if let Some(active) = status.get("world").and_then(Value::as_str) {
                bail!(
                    "le monde '{active}' tourne — la sauvegarde exige le mode setup \
                     (admin_shutdown_world d'abord)"
                );
            }
            let client = http()?;
            admin_auth(&client, &url, password).await?;
            let backup = run_backup(&client, &url, &pkg_type, &id, &note).await?;
            Ok(text_response(&backup))
        }

        "admin_shutdown_world" => {
            if args.get("confirm").and_then(Value::as_bool) != Some(true) {
                bail!("confirm:true requis — éteint le monde pour TOUT LE MONDE, bot compris");
            }
            let password = admin_password(state)?;
            let status = api_status(&url).await?;
            let world = status.get("world").and_then(Value::as_str);
            let Some(world) = world else {
                bail!("aucun monde actif — rien à éteindre");
            };
            *state.last_world_id.lock().await = Some(world.to_string());
            let client = http()?;
            let _ = client.get(format!("{url}/join")).send().await; // session
            let resp = client
                .post(format!("{url}/join"))
                .json(&json!({ "action": "shutdown", "adminPassword": password }))
                .send()
                .await
                .context("POST /join shutdown")?;
            let code = resp.status();
            let data: Value = resp.json().await.unwrap_or(Value::Null);
            if !code.is_success() {
                bail!("shutdown refusé (HTTP {code}) : {data}");
            }
            Ok(text_response(&json!({
                "shutdown": world,
                "note": "monde éteint — relancer avec admin_launch_world (le bot se reconnectera seul)",
                "response": data,
            })))
        }

        "admin_launch_world" => {
            let password = admin_password(state)?;
            let world =
                match str_arg(args, "world") {
                    Some(w) => w,
                    None => state.last_world_id.lock().await.clone().ok_or_else(|| {
                        anyhow!("précisez 'world' (aucun monde vu actif récemment)")
                    })?,
                };
            let status = api_status(&url).await?;
            if let Some(active) = status.get("world").and_then(Value::as_str) {
                bail!(
                    "le monde '{active}' tourne déjà — l'éteindre d'abord (admin_shutdown_world)"
                );
            }
            let client = http()?;
            admin_auth(&client, &url, password).await?;
            let data = setup_post(
                &client,
                &url,
                &json!({ "action": "launchWorld", "world": world }),
            )
            .await?;
            Ok(text_response(&json!({
                "launching": world,
                "note": "démarrage en cours (~10-30 s) — admin_status ou ping pour suivre ; le bot se reconnecte seul",
                "response": data,
            })))
        }

        "admin_check_package" | "admin_update_package" => {
            let pkg_type = str_arg(args, "type").unwrap_or_else(|| "module".into());
            let id = str_arg(args, "id").ok_or_else(|| anyhow!("'id' est requis"))?;
            let status = api_status(&url).await?;
            let world_active = status
                .get("world")
                .and_then(Value::as_str)
                .map(String::from);

            // Version installée + URL du manifest distant : lues sur le manifest
            // STATIQUE servi par l'instance — la réponse checkPackage ne contient
            // que le côté distant (vérifié en réel sur v13).
            let static_manifest = match pkg_type.as_str() {
                "module" => Some(format!("{url}/modules/{id}/module.json")),
                "system" => Some(format!("{url}/systems/{id}/system.json")),
                _ => None, // worlds : pas de manifest statique servi
            };
            let read_local = || async {
                let p = static_manifest.as_ref()?;
                let m: Value = reqwest::get(p).await.ok()?.json().await.ok()?;
                Some((
                    m.get("version").and_then(Value::as_str).map(String::from),
                    m.get("manifest").and_then(Value::as_str).map(String::from),
                ))
            };
            let (installed, manifest_url) = read_local().await.unwrap_or((None, None));

            if name == "admin_check_package" {
                // Check possible même monde allumé : le manifest distant se lit
                // directement à son URL. Monde éteint : checkPackage fait foi.
                let remote =
                    if world_active.is_some() {
                        let mu = manifest_url.clone().ok_or_else(|| {
                            anyhow!(
                                "manifest distant introuvable pour '{id}' — paquet installé ? \
                         (sinon, éteindre le monde pour un checkPackage complet)"
                            )
                        })?;
                        let m: Value = reqwest::get(&mu).await?.json().await?;
                        m.get("version").and_then(Value::as_str).map(String::from)
                    } else {
                        let client = http()?;
                        admin_auth(&client, &url, admin_password(state)?).await?;
                        let check = setup_post(&client, &url, &json!({
                        "action": "checkPackage", "strict": false, "type": pkg_type, "id": id,
                    }))
                    .await?;
                        check
                            .pointer("/remote/version")
                            .and_then(Value::as_str)
                            .map(String::from)
                    };
                let update_available =
                    matches!((&installed, &remote), (Some(a), Some(b)) if a != b);
                return Ok(text_response(&json!({
                    "package": id, "type": pkg_type,
                    "installed": installed, "remote": remote,
                    "updateAvailable": update_available,
                })));
            }

            // update : mode setup obligatoire (installPackage).
            if let Some(active) = world_active {
                bail!(
                    "le monde '{active}' tourne — la mise à jour exige le mode setup \
                     (admin_shutdown_world d'abord)"
                );
            }
            let client = http()?;
            admin_auth(&client, &url, admin_password(state)?).await?;
            let check = setup_post(
                &client,
                &url,
                &json!({
                    "action": "checkPackage", "strict": false, "type": pkg_type, "id": id,
                }),
            )
            .await?;
            let remote = check
                .pointer("/remote/version")
                .and_then(Value::as_str)
                .map(String::from);
            let manifest = check
                .pointer("/remote/manifest")
                .and_then(Value::as_str)
                .map(String::from)
                .or(manifest_url)
                .ok_or_else(|| anyhow!("pas de manifest distant pour '{id}'"))?;
            if installed.is_some() && installed == remote {
                return Ok(text_response(&json!({
                    "package": id, "installed": installed,
                    "updated": false, "reason": "déjà à jour",
                })));
            }
            // Filet : on sauvegarde AVANT de toucher au paquet (désactivable).
            let backup = if args.get("backup").and_then(Value::as_bool) != Some(false) {
                match run_backup(&client, &url, &pkg_type, &id, "avant mise à jour (MCP)").await {
                    Ok(b) => b.get("backupId").cloned().unwrap_or(Value::Null),
                    Err(e) => bail!(
                        "sauvegarde préalable échouée ({e:#}) — mise à jour ANNULÉE. \
                         Relancer avec backup:false pour passer outre."
                    ),
                }
            } else {
                Value::Null
            };
            // installPackage répond vite ; la fin réelle se vérifie sur le
            // manifest statique (la version change quand l'extraction est finie).
            // Un téléchargement distant peut échouer (timeout, réseau, miroir) :
            // 5 tentatives complètes avant d'abandonner.
            let mut verified = None;
            let mut attempts = 0;
            let mut last_error = String::new();
            'attempts: for attempt in 1..=5 {
                attempts = attempt;
                if attempt > 1 {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
                if let Err(e) = setup_post(&client, &url, &json!({
                    "action": "installPackage", "type": pkg_type, "id": id, "manifest": manifest,
                }))
                .await
                {
                    last_error = format!("tentative {attempt} : {e:#}");
                    continue;
                }
                // vérification : jusqu'à 2 min pour voir la nouvelle version
                for _ in 0..24 {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    if let Some((now, _)) = read_local().await
                        && now.is_some()
                        && now != installed
                    {
                        verified = now;
                        break 'attempts;
                    }
                }
                last_error = format!(
                    "tentative {attempt} : installation lancée mais non confirmée en 2 min"
                );
            }
            Ok(text_response(&json!({
                "package": id, "type": pkg_type,
                "from": installed, "to": remote,
                "installedNow": verified,
                "updated": verified.is_some(),
                "attempts": attempts,
                "backupId": backup,
                "note": if verified.is_some() { "vérifié installé (manifest statique)".to_string() }
                        else { format!("échec après {attempts} tentatives — {last_error}") },
            })))
        }
        other => bail!("Unknown tool: {other}"),
    }
}
